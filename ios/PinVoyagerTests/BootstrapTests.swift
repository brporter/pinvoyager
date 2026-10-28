import PinVoyagerData
import PinVoyagerDomain
import XCTest

@testable import PinVoyager

private actor WorkspaceOpenBarrier {
  let started: XCTestExpectation
  let store: any WorkspaceStore
  var pending: CheckedContinuation<Void, Never>?
  var opens = 0
  init(started: XCTestExpectation, store: any WorkspaceStore) {
    self.started = started
    self.store = store
  }
  func open() async -> any WorkspaceStore {
    opens += 1
    await withCheckedContinuation { continuation in
      pending = continuation
      started.fulfill()
    }
    return store
  }
  func release() {
    pending?.resume()
    pending = nil
  }
}
@MainActor private final class StartupMarker { var returned = false }

@MainActor final class BootstrapTests: XCTestCase {
  func testConcurrentStartupCallersWaitForTheSameStoreBeforeProceeding() async throws {
    let started = expectation(description: "Workspace opening is in progress")
    let barrier = WorkspaceOpenBarrier(
      started: started, store: try CoreDataWorkspaceStore(url: nil))
    let bootstrap = AppBootstrap(makeStore: { _ in await barrier.open() })
    let first = Task { await bootstrap.start() }
    await fulfillment(of: [started], timeout: 2)
    let joined = expectation(description: "Second caller joins startup")
    let marker = StartupMarker()
    let second = Task {
      joined.fulfill()
      await bootstrap.start()
      marker.returned = true
    }
    await fulfillment(of: [joined], timeout: 2)
    XCTAssertFalse(marker.returned)
    XCTAssertNil(bootstrap.model)
    await barrier.release()
    await first.value
    await second.value
    XCTAssertNotNil(bootstrap.model)
    XCTAssertTrue(marker.returned)
    let opens = await barrier.opens
    XCTAssertEqual(opens, 1)
  }
}

private actor RetryingWorkspaceOpen {
  let store: any WorkspaceStore
  var opens = 0
  init(store: any WorkspaceStore) { self.store = store }
  func open() throws -> any WorkspaceStore {
    opens += 1
    if opens == 1 { throw CocoaError(.fileReadCorruptFile) }
    return store
  }
}
extension BootstrapTests {
  func testFailedOpeningCanBeRetriedAndAnOpenedWorkspaceIsNotOpenedAgain() async throws {
    let plan = RetryingWorkspaceOpen(store: try CoreDataWorkspaceStore(url: nil))
    let bootstrap = AppBootstrap(makeStore: { _ in try await plan.open() })
    await bootstrap.start()
    XCTAssertNil(bootstrap.model)
    XCTAssertNotNil(bootstrap.error)
    await bootstrap.start()
    XCTAssertNotNil(bootstrap.model)
    XCTAssertNil(bootstrap.error)
    await bootstrap.start()
    let count = await plan.opens
    XCTAssertEqual(count, 2)
  }
}

extension BootstrapTests {
  func testBackgroundCompositionCheckpointsFailuresAndProvidesAccountScopedCoordinators()
    async throws
  {
    let state = workspaceWithPhotos(remote: true)
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(
      SessionCredential(accountId: state.account.id, token: "synthetic-session"))
    let origin = try testOrigin()
    let client = APIClient(origin: origin, transport: StubHTTP([]), credentials: credentials)
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let vault = try FileVault(root: root)
    let model = AppModel(store: store, client: client, credentials: credentials, vault: vault)
    await model.restore(accountId: state.account.id)
    let driver = SystemTransferDriver.shared
    let previous = driver.onCompletion
    defer { driver.onCompletion = previous }
    try await AppBootstrap().configureBackground(
      model: model, vault: vault, credentials: credentials, origin: origin)
    let capabilities = try APIJSON.decoder().decode(
      ClientCapabilities.self,
      from: Data(
        contentsOf: Fixtures.directory("api").appendingPathComponent("native_capabilities.json")))
    XCTAssertNotNil(model.makeCoordinator?(state.account.id, capabilities))
    let operation = OutboxOperation(
      accountId: state.account.id, generation: state.account.generation,
      voyageId: state.voyages[0].id,
      kind: .upload(photoId: state.photos[0].id, publicConsent: false))
    let descriptor = TransferDescriptor(
      accountId: state.account.id, generation: state.account.generation, operationId: operation.id)
    try await store.update(accountId: state.account.id) { current in
      var operation = operation
      operation.state = .submitted
      operation.request = MutationRequest(method: "POST", path: "/synthetic-upload")
      current.operations.append(operation)
      var transfer = BackgroundTransfer(
        operationId: operation.id, generation: descriptor.generation,
        sessionIdentifier: descriptor.sessionIdentifier, bodyPath: "synthetic-body", byteCount: 1)
      transfer.taskId = 123
      current.transfers.append(transfer)
    }
    let callback = try XCTUnwrap(driver.onCompletion)
    try await callback(
      TransferCompletion(
        descriptor: descriptor, taskId: 123,
        response: HTTPResponse(
          status: 415, headers: ["x-request-id": "background-reference"], body: Data())))
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertEqual(saved.operations.first { $0.id == operation.id }?.state, .attention)
    XCTAssertEqual(
      saved.operations.first { $0.id == operation.id }?.requestId, "background-reference")
    XCTAssertEqual(saved.photos.first { $0.id == state.photos[0].id }?.transfer, .needsAttention)
    XCTAssertNotNil(model.cancelPersistedTransfers)
  }
}

private actor ObservedStoreOpen {
  let store: any WorkspaceStore
  var urls: [URL?] = []
  init(store: any WorkspaceStore) { self.store = store }
  func open(_ url: URL?) -> any WorkspaceStore {
    urls.append(url)
    return store
  }
}

extension BootstrapTests {
  func testProductionStartupRestoresItsAccountAndReconnectsUsingInjectedServices() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let observed = ObservedStoreOpen(store: store)
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    var environment = try AppEnvironment.system(testing: false)
    environment.directory = root
    environment.cacheDirectory = root.appendingPathComponent("cache")
    environment.credentials = TestCredentials()
    environment.deletionRecovery = MemoryDeletionRecovery()
    environment.preferences = UserDefaultsDevicePreferences(
      suiteName: "pinvoyager.startup.\(UUID())")
    environment.transport = StubHTTP([])
    environment.activeAccountId = state.account.id
    environment.transfers = SystemTransferDriver(evidenceRoot: root.appendingPathComponent("files"))
    let bootstrap = AppBootstrap(environment: environment, makeStore: { await observed.open($0) })
    await bootstrap.start()
    XCTAssertNil(bootstrap.error)
    XCTAssertEqual(bootstrap.model?.activeAccountId, state.account.id)
    XCTAssertNotNil(bootstrap.model?.makeCoordinator)
    let urls = await observed.urls
    XCTAssertEqual(urls, [root.appendingPathComponent("workspace.sqlite")])
    await bootstrap.start()
    let count = await observed.urls.count
    XCTAssertEqual(count, 1)
    XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent("files").path))
  }
  func testInvalidConfiguredOriginProducesARecoverableStartupError() async throws {
    var environment = try AppEnvironment.system(testing: true)
    environment.apiURL = URL(fileURLWithPath: "/not-a-web-origin")
    let bootstrap = AppBootstrap(
      environment: environment, makeStore: { _ in try CoreDataWorkspaceStore(url: nil) })
    await bootstrap.start()
    XCTAssertNil(bootstrap.model)
    XCTAssertNotNil(bootstrap.error)
  }
}
