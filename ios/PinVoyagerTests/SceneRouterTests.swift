import PinVoyagerData
import PinVoyagerDomain
import XCTest

@testable import PinVoyager

@MainActor final class SceneRouterTests: XCTestCase {
  private func model(_ state: Workspace? = nil, transport: any HTTPTransport = StubHTTP([]))
    async throws -> AppModel
  {
    let store = try CoreDataWorkspaceStore(url: nil)
    if let state { try await store.insert(state) }
    let credentials = TestCredentials()
    if let state {
      await credentials.save(
        SessionCredential(accountId: state.account.id, token: "synthetic-route-session"))
    }
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: transport, credentials: credentials), credentials: credentials)
    await model.restore(accountId: state?.account.id)
    return model
  }

  func testWindowsKeepIndependentRoutesWhileSharingOneWorkspace() async throws {
    let state = workspaceWithPhotos()
    let model = try await model(state)
    let first = SceneRouter()
    let second = SceneRouter()
    await first.handle(URL(string: "https://pinvoyager.com/upload")!, model: model)
    XCTAssertTrue(first.showingCreation)
    XCTAssertFalse(second.showingCreation)
    await second.handle(URL(string: "https://pinvoyager.com/settings")!, model: model)
    XCTAssertEqual(second.selectedTab, .account)
    XCTAssertEqual(first.selectedTab, .discover)
    first.voyagePath = [.localVoyage(state.voyages[0].id)]
    first.syncPath = [.voyageSync(state.voyages[0].id)]
    first.accountPath = [.preferences, .help]
    let restored = SceneRouter()
    restored.restore(first.restorationData(accountId: state.account.id), workspace: state)
    XCTAssertEqual(restored.voyagePath, first.voyagePath)
    XCTAssertEqual(restored.syncPath, first.syncPath)
    XCTAssertEqual(restored.accountPath, first.accountPath)
    let otherAccount = unlockedWorkspace()
    restored.restore(first.restorationData(accountId: state.account.id), workspace: otherAccount)
    XCTAssertTrue(restored.voyagePath.isEmpty)
    XCTAssertTrue(restored.syncPath.isEmpty)
    XCTAssertTrue(restored.accountPath.isEmpty)
  }

  func testPendingAuthenticationRouteSurvivesRestorationAndCancellationClearsIt() async throws {
    let model = try await model()
    let first = SceneRouter()
    await first.handle(URL(string: "https://evil.test/upload")!, model: model)
    XCTAssertFalse(first.showingSignIn)
    await first.handle(URL(string: "https://pinvoyager.com/upload")!, model: model)
    XCTAssertEqual(first.pendingRoute, .createVoyage)
    let second = SceneRouter()
    second.restore(first.restorationData(accountId: nil), workspace: nil)
    XCTAssertEqual(second.pendingRoute, .createVoyage)
    XCTAssertTrue(second.showingSignIn)
    second.cancelAuthentication()
    XCTAssertNil(second.pendingRoute)
    XCTAssertFalse(second.showingSignIn)
    second.restore(Data("damaged".utf8), workspace: nil)
    XCTAssertEqual(second.selectedTab, .discover)
  }

  func testDeepLinkWaitsForDurableEditorSavingAndRetainsTheLinkWhenSavingFails() async throws {
    let model = try await model()
    let router = SceneRouter()
    let id = UUID()
    let link = URL(string: "https://pinvoyager.com/voyages/\(id)")!
    router.beforeNavigation = { throw CocoaError(.fileWriteOutOfSpace) }
    await router.handle(link, model: model)
    XCTAssertTrue(router.discoverPath.isEmpty)
    XCTAssertEqual(router.deferredRoute, .voyage(id))
    XCTAssertNotNil(model.error)
    var saved = false
    router.beforeNavigation = { saved = true }
    await router.retryNavigation(model: model)
    XCTAssertTrue(saved)
    XCTAssertEqual(router.discoverPath, [.voyage(id)])
    XCTAssertNil(router.deferredRoute)
    router.accountChanged(from: UUID(), to: nil)
    XCTAssertTrue(router.discoverPath.isEmpty)
  }
}

extension SceneRouterTests {
  func testOwnerLinksOpenTheMappedLocalEditorOrImportOnlyAfterOwnershipIsVerified() async throws {
    for importing in [false, true] {
      var state = workspaceWithPhotos()
      var detail = try XCTUnwrap(
        try JSONSerialization.jsonObject(
          with: Data(
            contentsOf: Fixtures.directory("api").appendingPathComponent("voyage_detail.json")))
          as? [String: Any])
      detail["owner_id"] = state.account.id.uuidString
      let remoteId = try XCTUnwrap(UUID(uuidString: try XCTUnwrap(detail["id"] as? String)))
      state.voyages[0].serverId = remoteId
      let transport = StubHTTP([
        HTTPResponse(status: 200, body: try JSONSerialization.data(withJSONObject: detail)),
        HTTPResponse(status: 200, body: Data("{\"voyages\":[],\"next_before\":null}".utf8)),
      ])
      let model = try await model(state, transport: transport)
      let router = SceneRouter()
      let path = importing ? "upload?voyage=\(remoteId)" : "voyages/\(remoteId)/edit"
      await router.handle(URL(string: "https://pinvoyager.com/\(path)")!, model: model)
      XCTAssertEqual(router.selectedTab, .voyages)
      XCTAssertEqual(router.voyagePath, [.localVoyage(state.voyages[0].id)])
      XCTAssertEqual(
        router.ownerAction, importing ? .addPhotos(state.voyages[0].id) : .edit(state.voyages[0].id)
      )
      XCTAssertNil(model.error)
    }
  }

  func testOwnerLinksPreserveGuestIntentAndRejectForeignOrUnmappedVoyages() async throws {
    let guest = try await model()
    let router = SceneRouter()
    let id = UUID()
    await router.handle(URL(string: "https://pinvoyager.com/voyages/\(id)/edit")!, model: guest)
    XCTAssertEqual(router.pendingRoute, .editVoyage(id))
    await router.resumePendingRoute(model: guest)
    XCTAssertTrue(router.showingSignIn)
    for owned in [false, true] {
      let state = workspaceWithPhotos()
      var detail = try XCTUnwrap(
        try JSONSerialization.jsonObject(
          with: Data(
            contentsOf: Fixtures.directory("api").appendingPathComponent("voyage_detail.json")))
          as? [String: Any])
      detail["owner_id"] = (owned ? state.account.id : UUID()).uuidString
      let model = try await model(
        state,
        transport: StubHTTP([
          HTTPResponse(status: 200, body: try JSONSerialization.data(withJSONObject: detail)),
          HTTPResponse(status: 200, body: Data("{\"voyages\":[],\"next_before\":null}".utf8)),
        ]))
      await router.handle(URL(string: "https://pinvoyager.com/voyages/\(id)/edit")!, model: model)
      XCTAssertNotNil(model.error)
      XCTAssertNil(router.ownerAction)
    }
  }

  func testRestorationDropsMissingLocalDestinationsAndUnknownVersionsWithoutChangingOtherWindows()
    throws
  {
    let state = workspaceWithPhotos()
    let source = SceneRouter()
    let restored = SceneRouter()
    source.discoverPath = [.voyage(UUID()), .preferences, .profile("alice")]
    source.voyagePath = [.localVoyage(UUID()), .exportVoyage(state.voyages[0].id), .help]
    source.syncPath = [.voyageSync(UUID())]
    source.pendingRoute = .createVoyage
    source.deferredRoute = .account
    let data = source.restorationData(accountId: state.account.id)
    restored.restore(data, workspace: state)
    XCTAssertEqual(restored.discoverPath.count, 2)
    XCTAssertEqual(restored.voyagePath, [.exportVoyage(state.voyages[0].id), .help])
    XCTAssertTrue(restored.syncPath.isEmpty)
    XCTAssertEqual(restored.deferredRoute, .account)
    var saved = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    saved["version"] = 999
    restored.restore(try JSONSerialization.data(withJSONObject: saved), workspace: state)
    XCTAssertEqual(restored.deferredRoute, .account)
    restored.accountChanged(from: state.account.id, to: state.account.id)
    XCTAssertEqual(restored.pendingRoute, .createVoyage)
    restored.sheetDismissed()
    XCTAssertFalse(restored.showingSignIn)
    restored.signInAfterDismiss = true
    restored.sheetDismissed()
    XCTAssertTrue(restored.showingSignIn)
  }
}

extension SceneRouterTests {
  func testCreationOpensImportOnlyAfterTheEditorDismissesAndCannotCrossAccounts() {
    let router = SceneRouter()
    let voyageId = UUID()
    router.requestCreation(hasWorkspace: true)
    router.creationCompleted(voyageId: voyageId, addPhotos: true)
    XCTAssertEqual(router.selectedTab, .voyages)
    XCTAssertTrue(router.voyagePath.isEmpty)
    XCTAssertNil(router.ownerAction)
    router.creationDismissed()
    XCTAssertEqual(router.voyagePath, [.localVoyage(voyageId)])
    XCTAssertEqual(router.ownerAction, .addPhotos(voyageId))
    router.creationDismissed()
    XCTAssertEqual(router.voyagePath.count, 1)

    let savedAndClosed = SceneRouter()
    savedAndClosed.creationCompleted(voyageId: voyageId, addPhotos: false)
    savedAndClosed.creationDismissed()
    XCTAssertEqual(savedAndClosed.selectedTab, .voyages)
    XCTAssertTrue(savedAndClosed.voyagePath.isEmpty)
    XCTAssertNil(savedAndClosed.ownerAction)

    let switched = SceneRouter()
    switched.creationCompleted(voyageId: voyageId, addPhotos: true)
    switched.accountChanged(from: UUID(), to: UUID())
    switched.creationDismissed()
    XCTAssertTrue(switched.voyagePath.isEmpty)
    XCTAssertNil(switched.ownerAction)
  }

  func testCreateButtonRetainsItsIntentUntilSignInAndNameSetupFinish() async throws {
    let model = try await model()
    let router = SceneRouter()
    router.requestCreation(hasWorkspace: false)
    XCTAssertTrue(router.showingSignIn)
    XCTAssertEqual(router.pendingRoute, .createVoyage)
    var state = unlockedWorkspace()
    state.account.profile.displayName = nil
    try await model.store.insert(state)
    await model.restore(accountId: state.account.id)
    await router.resumePendingRoute(model: model)
    XCTAssertTrue(router.showingNameSetup)
    XCTAssertEqual(router.pendingRoute, .createVoyage)
    try await model.store.update(accountId: state.account.id) {
      $0.account.profile.displayName = "chosen-name"
    }
    await model.reloadWorkspace()
    await router.resumePendingRoute(model: model)
    XCTAssertTrue(router.showingCreation)
    XCTAssertNil(router.pendingRoute)
    XCTAssertFalse(router.showingNameSetup)
  }
}
