import PinVoyagerData
import PinVoyagerDomain
import XCTest

@testable import PinVoyager

@MainActor
final class AppModelTests: XCTestCase {
  func testSavingLocallySchedulesSynchronizationWithoutAnotherUserAction() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: state.account.id)
    model.capabilities = try APIJSON.decoder().decode(
      ClientCapabilities.self,
      from: Data(
        contentsOf: Fixtures.directory("api").appendingPathComponent("native_capabilities.json")))
    let sent = expectation(description: "Local save schedules the outbox")
    let remoteId = UUID()
    let remote = AutomaticSyncRemote(targetId: remoteId, submitted: sent)
    model.makeCoordinator = { _, _ in SyncCoordinator(store: store, remote: remote) }
    let localId = try await model.create(
      VoyageFields(title: "Automatic", description: "Saved first"))
    XCTAssertEqual(model.workspace?.voyages.first?.id, localId)
    await fulfillment(of: [sent], timeout: 2)
    await model.sync()
    XCTAssertEqual(model.workspace?.voyages.first?.serverId, remoteId)
  }

  func testNewSessionInvalidatesOldTransferGenerationWithoutChangingOperationIdentity() async throws
  {
    var state = workspaceWithPhotos()
    state.account.access = .requiresSignIn
    state.account.generation = 4
    let operation = OutboxOperation(
      accountId: state.account.id, generation: 4, voyageId: state.voyages[0].id,
      kind: .upload(photoId: state.photos[0].id, publicConsent: false))
    state.operations = [operation]
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    let id = state.account.id
    let session = try APIJSON.decoder().decode(
      SessionExchange.self,
      from: Data(
        "{\"session_token\":\"new-session\",\"user\":{\"id\":\"\(id)\",\"display_name\":\"traveler\"},\"identities\":[],\"idle_deadline\":\"2026-11-01T00:00:00Z\",\"absolute_deadline\":\"2026-12-01T00:00:00Z\"}"
          .utf8))
    try await model.acceptSession(session)
    let saved = try await store.read(accountId: id)
    XCTAssertEqual(saved.account.generation, 5)
    XCTAssertEqual(saved.operations.first?.id, operation.id)
    XCTAssertEqual(
      saved.photos.first { $0.id == state.photos[0].id }?.uploadKey, state.photos[0].uploadKey)
  }

  func testBlockReconciliationPreservesPendingChoicesAndAppliesWebsiteChanges() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let pending = UUID()
    let remote = UUID()
    try await LocalRepository(store: store).setBlocked(
      accountId: state.account.id, userId: pending, blocked: true)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "session"))
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([
          response(200, "{\"user_ids\":[\"\(remote)\"]}"), response(200, "{\"user_ids\":[]}"),
        ]), credentials: credentials), credentials: credentials)
    await model.restore(accountId: state.account.id)
    await model.reconcileBlocks()
    XCTAssertEqual(model.workspace?.blockedAccounts, Set([pending, remote]))
    try await store.update(accountId: state.account.id) { state in
      for index in state.operations.indices { state.operations[index].state = .applied }
    }
    await model.reconcileBlocks()
    XCTAssertEqual(model.workspace?.blockedAccounts, [])
  }

  func testInitialNameSetupPreservesTheIntendedCreationRoute() async throws {
    let account = UUID()
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: account, token: "session"))
    let me =
      "{\"user\":{\"id\":\"\(account)\",\"display_name\":\"new-traveler\"},\"identities\":[]}"
    let model = AppModel(
      store: try CoreDataWorkspaceStore(url: nil),
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([response(204, ""), response(200, me)]),
        credentials: credentials), credentials: credentials)
    let router = SceneRouter()
    await router.handle(URL(string: "https://pinvoyager.com/upload")!, model: model)
    XCTAssertEqual(router.pendingRoute, .createVoyage)
    let session = try APIJSON.decoder().decode(
      SessionExchange.self,
      from: Data(
        "{\"session_token\":\"session\",\"user\":{\"id\":\"\(account)\",\"display_name\":null},\"identities\":[],\"idle_deadline\":\"2026-11-01T00:00:00Z\",\"absolute_deadline\":\"2026-12-01T00:00:00Z\"}"
          .utf8))
    try await model.acceptSession(session)
    await router.resumePendingRoute(model: model)
    XCTAssertTrue(router.showingNameSetup)
    XCTAssertFalse(router.showingCreation)
    try await model.saveDisplayName("New-Traveler")
    await router.resumePendingRoute(model: model)
    XCTAssertFalse(router.showingNameSetup)
    XCTAssertTrue(router.showingCreation)
    XCTAssertNil(router.pendingRoute)
    XCTAssertEqual(model.workspace?.account.profile.displayName, "new-traveler")
  }

  func testDeletionRecoveryLocksTheWorkspaceAfterConfirmedRemoval() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let recovery = MemoryDeletionRecovery()
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "session"))
    let first = APIClient(
      origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials)
    _ = try? await AccountDeletionService(client: first, recovery: recovery).prepare(
      accountId: state.account.id, proofToken: "proof")
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([response(200, "{\"status\":\"complete\"}"), response(204, "")]),
        credentials: credentials), credentials: credentials, deletionRecovery: recovery)
    await model.restore(accountId: state.account.id)
    await model.refreshDeletion(accountId: state.account.id)
    XCTAssertNil(model.workspace)
    XCTAssertNil(model.activeAccountId)
    XCTAssertEqual(model.pendingDeletions.first?.status, .complete)
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertEqual(saved.account.access, .locked)
  }

  func testOwnerDownloadRequiresConnectionAndKeepsLocalWork() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: state.account.id)
    model.changeNetwork(.offline)
    await model.downloadOwnerCopy(UUID())
    XCTAssertNotNil(model.error)
    XCTAssertEqual(model.workspace?.account.id, state.account.id)
    XCTAssertTrue(model.downloadingVoyages.isEmpty)
  }

  func testLoadsSavedOwnerWorkspaceWithoutNetwork() async throws {
    let store = try CoreDataWorkspaceStore(url: nil)
    let state = unlockedWorkspace()
    try await store.insert(state)
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: TestCredentials()),
      credentials: TestCredentials())
    await model.restore(accountId: state.account.id)
    XCTAssertEqual(model.workspace?.account.id, state.account.id)
    XCTAssertNil(model.error)
  }

  func testLockedAccountNeverAppearsAsSavedSummary() async throws {
    let store = try CoreDataWorkspaceStore(url: nil)
    var state = unlockedWorkspace()
    state.account.access = .locked
    try await store.insert(state)
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: TestCredentials()),
      credentials: TestCredentials())
    await model.restore(accountId: state.account.id)
    XCTAssertNil(model.workspace)
  }

  func testPaginationDeduplicatesAndKeepsRowsOnFailure() async throws {
    let fixtures = Fixtures.directory("api").appendingPathComponent("voyage_list.json")
    let body = try Data(contentsOf: fixtures)
    let transport = StubHTTP([
      HTTPResponse(status: 200, body: body), HTTPResponse(status: 200, body: body),
    ])
    let client = APIClient(
      origin: try testOrigin(), transport: transport, credentials: TestCredentials())
    let model = AppModel(
      store: try CoreDataWorkspaceStore(url: nil), client: client, credentials: TestCredentials())
    await model.loadDiscover(refresh: true)
    let count = model.discover.count
    await model.loadDiscover(refresh: false)
    XCTAssertEqual(model.discover.count, count)
    await model.loadDiscover(refresh: true)
    XCTAssertEqual(model.discover.count, count)
    XCTAssertNotNil(model.discoverError)
  }

  func testLocalCommandsAndOfflinePublicDataPolicy() async throws {
    let store = try CoreDataWorkspaceStore(url: nil)
    let state = unlockedWorkspace()
    try await store.insert(state)
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: TestCredentials()),
      credentials: TestCredentials())
    await model.restore(accountId: state.account.id)
    let id = try await model.create(VoyageFields(title: "First", description: "Text"))
    XCTAssertEqual(model.localVoyages.count, 1)
    try await model.edit(id, fields: VoyageFields(title: "Changed", description: nil))
    XCTAssertEqual(model.localVoyages[0].fields.title, "Changed")
    XCTAssertTrue(model.signedIn)
    model.changeNetwork(.offline)
    await model.loadDiscover(refresh: true)
    XCTAssertNotNil(model.discoverError)
    XCTAssertTrue(model.discover.isEmpty)
    XCTAssertEqual(model.workspace?.voyages.count, 1)
    await model.perform { throw WorkspaceError.invalidOrder }
    XCTAssertNotNil(model.error)
  }

  func testExpiredSessionsPauseRemoteWorkAndKeepPreviouslyVerifiedLocalAuthoring() async throws {
    let store = try CoreDataWorkspaceStore(url: nil)
    let state = unlockedWorkspace()
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
    let offline = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await offline.restore(accountId: state.account.id)
    await offline.verifyAccount()
    XCTAssertNotNil(offline.workspace)
    let rejected = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([response(401, "")]), credentials: credentials
      ), credentials: credentials)
    await rejected.restore(accountId: state.account.id)
    await rejected.verifyAccount()
    XCTAssertNotNil(rejected.workspace)
    XCTAssertFalse(rejected.signedIn)
    let local = try await rejected.create(
      VoyageFields(title: "Offline after expiry", description: "Still saved locally"))
    XCTAssertTrue(rejected.workspace?.voyages.contains { $0.id == local } == true)
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertEqual(saved.account.access, .requiresSignIn)
  }

  func testVerifiedAccountRefreshesOnlySameAccount() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
    let accountJSON =
      "{\"user\":{\"id\":\"\(state.account.id)\",\"display_name\":\"updated\"},\"identities\":[]}"
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([response(200, accountJSON)]),
        credentials: credentials), credentials: credentials)
    await model.restore(accountId: state.account.id)
    await model.verifyAccount()
    XCTAssertEqual(model.workspace?.account.profile.displayName, "updated")
    XCTAssertNil(model.error)
  }

  func testSessionSwitchLocksPreviousWorkspaceAndSignOutRetainsFiles() async throws {
    let store = try CoreDataWorkspaceStore(url: nil)
    let first = workspaceWithPhotos()
    try await store.insert(first)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: first.account.id)
    let secondId = UUID()
    let session = try APIJSON.decoder().decode(
      SessionExchange.self,
      from: Data(
        "{\"session_token\":\"token\",\"user\":{\"id\":\"\(secondId)\",\"display_name\":\"second\"},\"identities\":[],\"idle_deadline\":\"date\",\"absolute_deadline\":\"date\"}"
          .utf8))
    try await model.acceptSession(session)
    let locked = try await store.read(accountId: first.account.id)
    XCTAssertEqual(locked.account.access, .locked)
    XCTAssertEqual(model.workspace?.account.id, secondId)
    await credentials.save(SessionCredential(accountId: secondId, token: "token"))
    model.changeNetwork(.offline)
    await model.signOut()
    XCTAssertNil(model.workspace)
    XCTAssertNil(model.activeAccountId)
    let revoked = await credentials.read(accountId: secondId)
    XCTAssertEqual(revoked?.revocationOnly, true)
    let retained = try await store.read(accountId: first.account.id)
    XCTAssertEqual(retained.photos.count, 3)
  }

  func testDeepLinksPreserveValidationAndRequireSignInForCreation() async throws {
    let model = AppModel(
      store: try CoreDataWorkspaceStore(url: nil),
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: TestCredentials()),
      credentials: TestCredentials())
    let id = UUID()
    let router = SceneRouter()
    await router.handle(URL(string: "https://evil.test/upload")!, model: model)
    XCTAssertFalse(router.showingSignIn)
    await router.handle(URL(string: "https://pinvoyager.com/upload")!, model: model)
    XCTAssertTrue(router.showingSignIn)
    await router.handle(URL(string: "https://pinvoyager.com/voyages/\(id)")!, model: model)
    XCTAssertEqual(router.discoverPath, [.voyage(id)])
    await router.handle(URL(string: "https://pinvoyager.com/u/alice")!, model: model)
    XCTAssertEqual(router.discoverPath.last, .profile("alice"))
    await router.handle(URL(string: "https://pinvoyager.com/settings")!, model: model)
    XCTAssertEqual(router.selectedTab, .account)
  }

  func testOwnerReconciliationDoesNotOverwritePendingEditsOrInferDeletion() async throws {
    let fixtures = Fixtures.directory("api")
    let body = try Data(contentsOf: fixtures.appendingPathComponent("voyage_list_mine.json"))
    let page = try APIJSON.decoder().decode(VoyagePage.self, from: body)
    let accountId = try XCTUnwrap(page.voyages.first?.ownerId)
    var state = Workspace(
      account: LocalAccount(profile: UserProfile(id: accountId, displayName: "alice")))
    let remote = try XCTUnwrap(page.voyages.first)
    var local = LocalVoyage(
      ownerId: accountId, fields: VoyageFields(title: "Unsynced local name", description: "Local"))
    local.serverId = remote.id
    local.revision = "1"
    state.voyages.append(local)
    state.operations.append(
      OutboxOperation(
        accountId: accountId, generation: 0, voyageId: local.id, kind: .edit(local.fields),
        expectedRevision: "1"))
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: accountId, token: "token"))
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([HTTPResponse(status: 200, body: body)]),
        credentials: credentials), credentials: credentials)
    await model.restore(accountId: accountId)
    await model.refreshOwnerList()
    XCTAssertEqual(
      model.workspace?.voyages.first { $0.id == local.id }?.fields.title, "Unsynced local name")
    XCTAssertTrue(model.workspace?.voyages.contains { $0.id == local.id } == true)
  }

  func testRemoteOwnerPhotosUsePrivateMetadataAndNeverClaimDownloadedOriginals() async throws {
    let detail = try Data(
      contentsOf: Fixtures.directory("api").appendingPathComponent("voyage_detail.json"))
    let photos = try Data(
      contentsOf: Fixtures.directory("api").appendingPathComponent("photo_list.json"))
    let remote = try APIJSON.decoder().decode(RemoteVoyage.self, from: detail)
    var state = Workspace(
      account: LocalAccount(profile: UserProfile(id: remote.ownerId, displayName: "alice")))
    var local = LocalVoyage(ownerId: remote.ownerId, fields: remote.fields)
    local.serverId = remote.id
    state.voyages.append(local)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([
          HTTPResponse(status: 200, body: detail), HTTPResponse(status: 200, body: photos),
        ]), credentials: credentials), credentials: credentials)
    await model.restore(accountId: state.account.id)
    await model.refreshVoyage(local.id)
    XCTAssertEqual(model.workspace?.photos.count, 3)
    XCTAssertTrue(model.workspace?.photos.allSatisfy { $0.source == nil } == true)
    XCTAssertEqual(model.workspace?.photos.filter { $0.coordinate != nil }.count, 2)
  }
}

private actor AutomaticSyncRemote: SyncRemote {
  let targetId: UUID
  let submitted: XCTestExpectation
  init(targetId: UUID, submitted: XCTestExpectation) {
    self.targetId = targetId
    self.submitted = submitted
  }
  func supportsNativeSync() -> Bool { true }
  func receipt(operationId: UUID) -> OperationReceipt { .notSeen }
  func submit(_ request: MutationRequest, operationId: UUID) -> OperationReceipt {
    submitted.fulfill()
    return .applied(targetId: targetId, revision: "1")
  }
}

extension AppModelTests {
  func testPublicAccessChangesOnAccountReplacementSignOutAndForegroundValidation() async throws {
    let credentials = TestCredentials()
    let model = AppModel(
      store: try CoreDataWorkspaceStore(url: nil),
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    func session(_ id: UUID) throws -> SessionExchange {
      try APIJSON.decoder().decode(
        SessionExchange.self,
        from: Data(
          """
          {"session_token":"new-session","user":{"id":"\(id)","display_name":"traveler"},"identities":[],"idle_deadline":"2026-11-01T00:00:00Z","absolute_deadline":"2026-12-01T00:00:00Z"}
          """.utf8))
    }
    let signedOut = model.publicAccess
    let account = UUID()
    try await model.acceptSession(try session(account))
    let signedIn = model.publicAccess
    XCTAssertNotEqual(signedOut, signedIn)
    try await model.acceptSession(try session(account))
    XCTAssertNotEqual(signedIn, model.publicAccess)
    let beforeValidation = model.publicAccess
    await model.activate()
    XCTAssertNotEqual(beforeValidation, model.publicAccess)
    let beforeSignOut = model.publicAccess
    await model.signOut()
    XCTAssertNotEqual(beforeSignOut, model.publicAccess)
  }
}

private actor DelayedVerification: HTTPTransport {
  let started: XCTestExpectation
  var continuation: CheckedContinuation<HTTPResponse, Never>?
  init(started: XCTestExpectation) { self.started = started }
  func send(_ request: URLRequest) async -> HTTPResponse {
    await withCheckedContinuation { continuation in
      self.continuation = continuation
      started.fulfill()
    }
  }
  func rejectOldSession() {
    continuation?.resume(returning: response(401, "{}"))
    continuation = nil
  }
}

extension AppModelTests {
  func testAnOldVerificationFailureCannotLockARenewedSession() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "old-session"))
    let started = expectation(description: "Old verification is in flight")
    let transport = DelayedVerification(started: started)
    let model = AppModel(
      store: store,
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: state.account.id)
    let checking = Task { await model.verifyAccount() }
    await fulfillment(of: [started], timeout: 2)
    let session = try APIJSON.decoder().decode(
      SessionExchange.self,
      from: Data(
        """
        {"session_token":"new-session","user":{"id":"\(state.account.id)","display_name":"traveler"},"identities":[],"idle_deadline":"2026-11-01T00:00:00Z","absolute_deadline":"2026-12-01T00:00:00Z"}
        """.utf8))
    try await model.acceptSession(session)
    await transport.rejectOldSession()
    await checking.value
    XCTAssertTrue(model.signedIn)
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertEqual(saved.account.access, .unlocked)
    XCTAssertEqual(model.workspace?.account.generation, 1)
  }
}

extension AppModelTests {
  func testConfirmingDeletionPausesAndCancelsTransfersEvenWhenTheResponseIsLost() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
    let recovery = MemoryDeletionRecovery()
    let record = try JSONDecoder().decode(
      PendingAccountDeletion.self,
      from: JSONSerialization.data(withJSONObject: [
        "id": UUID().uuidString, "accountId": state.account.id.uuidString,
        "statusCapability": "synthetic-recovery-capability",
        "status": "prepared",
        "expiresAt": Date().addingTimeInterval(600).timeIntervalSinceReferenceDate,
      ]))
    await recovery.save(record)
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([
          response(503, "{}"), response(200, "{\"status\":\"prepared\"}"),
        ]), credentials: credentials), credentials: credentials, deletionRecovery: recovery)
    await model.restore(accountId: state.account.id)
    var canceled = false
    model.cancelPersistedTransfers = { accountId in
      XCTAssertEqual(accountId, state.account.id)
      canceled = true
    }
    await model.confirmAccountDeletion()
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertTrue(saved.policy.paused)
    XCTAssertEqual(saved.account.generation, 1)
    XCTAssertTrue(canceled)
    XCTAssertEqual(saved.account.access, .requiresSignIn)
  }
}

extension AppModelTests {
  func testUnknownAccountDeletionKeepsTheWorkspaceLockedAndShowsItsUncertainStatus() async throws {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
    let recovery = MemoryDeletionRecovery()
    let record = try JSONDecoder().decode(
      PendingAccountDeletion.self,
      from: JSONSerialization.data(withJSONObject: [
        "id": UUID().uuidString, "accountId": state.account.id.uuidString,
        "statusCapability": "synthetic-status-only-capability",
        "status": "prepared",
        "expiresAt": Date().addingTimeInterval(600).timeIntervalSinceReferenceDate,
      ]))
    await recovery.save(record)
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials, deletionRecovery: recovery)
    await model.restore(accountId: state.account.id)
    await model.confirmAccountDeletion()
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertEqual(saved.account.access, .locked)
    XCTAssertTrue(saved.policy.paused)
    XCTAssertNil(model.workspace)
    XCTAssertEqual(model.pendingDeletions.first?.status, .confirming)
  }
}

extension AppModelTests {
  func testBlockingImmediatelyHidesDiscoveryCardsAndKeepsMapAuthorizationPendingUntilAcknowledged()
    async throws
  {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "session"))
    let list = try Data(
      contentsOf: Fixtures.directory("api").appendingPathComponent("voyage_list.json"))
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([HTTPResponse(status: 200, body: list)]),
        credentials: credentials), credentials: credentials)
    await model.restore(accountId: state.account.id)
    await model.loadDiscover(refresh: true)
    let owner = try XCTUnwrap(model.discover.first?.ownerId)
    await model.perform {
      try await model.repository.setBlocked(
        accountId: state.account.id, userId: owner, blocked: true)
    }
    XCTAssertFalse(model.discover.contains { $0.ownerId == owner })
    XCTAssertTrue(model.publicAccess.pendingBlocks)
    let pending = model.publicAccess
    try await store.update(accountId: state.account.id) { state in
      for index in state.operations.indices { state.operations[index].state = .applied }
    }
    await model.reloadWorkspace()
    XCTAssertFalse(model.publicAccess.pendingBlocks)
    XCTAssertNotEqual(model.publicAccess, pending)
  }
}

extension AppModelTests {
  func testPublicationNotificationPreferenceAndDeliveryFollowConfirmedSynchronization() async throws
  {
    var state = workspaceWithPhotos(remote: true)
    var intent = try PublicationIntent.review(
      voyage: state.voyages[0], photos: state.photos, at: Date())
    intent.serverId = UUID()
    intent.status = .processing
    state.publicationIntents = [intent]
    var operation = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
      kind: .publish(intentId: intent.id))
    operation.state = .applied
    state.operations = [operation]
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
    let notifications = TestPublicationNotifications()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials, notifications: notifications)
    await model.restore(accountId: state.account.id)
    model.capabilities = try APIJSON.decoder().decode(
      ClientCapabilities.self,
      from: Data(
        contentsOf: Fixtures.directory("api").appendingPathComponent("native_capabilities.json")))
    let remote = StubSyncRemote()
    await remote.setPublicationResult(
      .publication(
        intentId: intent.serverId!, status: .published, revision: "99", confirmedPublished: true))
    model.makeCoordinator = { _, _ in SyncCoordinator(store: store, remote: remote) }
    await model.setPublicationNotifications(true)
    XCTAssertEqual(model.workspace?.policy.notifyOnPublication, true)
    await model.sync()
    let deliveries = await notifications.deliveries
    XCTAssertEqual(deliveries, 1)
    XCTAssertTrue(model.workspace?.voyages[0].confirmedPublished == true)
    await model.setPublicationNotifications(false)
    await notifications.allow(false)
    await model.setPublicationNotifications(true)
    XCTAssertEqual(model.workspace?.policy.notifyOnPublication, false)
    XCTAssertTrue(model.error?.contains("Notification access is off") == true)
  }
}

extension AppModelTests {
  func testFullOwnerDownloadsReleaseOnlyEligibleOriginalsAfterExplicitStorageOptOut() async throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    var state = workspaceWithPhotos(remote: true)
    state.policy.keepOriginals = false
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let vault = try FileVault(root: directory)
    let original = try XCTUnwrap(state.photos[0].source)
    try await vault.write(
      Data([1, 2, 3]), relativePath: original.relativePath, accountId: state.account.id)
    let bytes = Data("RIFF1234WEBPstripped".utf8)
    let manifest = try downloadManifest(
      account: state.account.id, voyage: state.voyages[0].serverId!,
      photo: state.photos[0].serverId!, bytes: bytes, revision: "7")
    let file = HTTPResponse(
      status: 200, headers: ["x-pinvoyager-retention": "owner_offline_copy_v1"], body: bytes)
    let transport = StubHTTP([
      HTTPResponse(status: 200, body: manifest), file, file, file,
      HTTPResponse(status: 200, body: manifest),
    ])
    let credentials = TestCredentials()
    await credentials.save(
      SessionCredential(accountId: state.account.id, token: "synthetic-download-session"))
    let model = AppModel(
      store: store,
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials, vault: vault)
    await model.restore(accountId: state.account.id)
    await model.downloadFullCopy(state.photos[0].id)
    XCTAssertNil(model.error)
    XCTAssertFalse(model.downloadingPhotos.contains(state.photos[0].id))
    let saved = try await store.read(accountId: state.account.id)
    let downloaded = try XCTUnwrap(saved.photos.first { $0.id == state.photos[0].id })
    XCTAssertNotNil(downloaded.offlineCopy?.variants["full"])
    XCTAssertNil(downloaded.source)
    let originalURL = try await vault.url(original.relativePath, accountId: state.account.id)
    XCTAssertFalse(FileManager.default.fileExists(atPath: originalURL.path))
    XCTAssertNotNil(saved.photos.first { $0.id == state.photos[1].id }?.source)
  }
}

extension AppModelTests {
  func testExplicitOfflineSignOutDiscardCancelsBeforeRemovingOnlyTheSelectedAccountsFiles()
    async throws
  {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let state = workspaceWithPhotos()
    let other = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    try await store.insert(other)
    let vault = try FileVault(root: directory)
    try await vault.write(
      Data([1]), relativePath: "originals/kept-until-canceled", accountId: state.account.id)
    try await vault.write(
      Data([2]), relativePath: "originals/other-account", accountId: other.account.id)
    let credentials = TestCredentials()
    await credentials.save(
      SessionCredential(accountId: state.account.id, token: "revocation-only-after-signout"))
    await credentials.save(
      SessionCredential(accountId: other.account.id, token: "other-account-session"))
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials, vault: vault)
    await model.restore(accountId: state.account.id)
    model.changeNetwork(.offline)
    var canceled = false
    model.cancelPersistedTransfers = { id in
      let locked = try? await store.read(accountId: id)
      XCTAssertEqual(locked?.account.access, .locked)
      let source = try? await vault.read("originals/kept-until-canceled", accountId: id)
      XCTAssertEqual(source, Data([1]))
      canceled = true
    }
    await model.signOut(purge: true)
    XCTAssertTrue(canceled)
    XCTAssertNil(model.error)
    XCTAssertNil(model.workspace)
    XCTAssertNil(model.activeAccountId)
    let accounts = try await store.accountIds()
    XCTAssertEqual(accounts, [other.account.id])
    let revoked = await credentials.read(accountId: state.account.id)
    XCTAssertEqual(revoked?.revocationOnly, true)
    let otherSession = await credentials.read(accountId: other.account.id)
    XCTAssertEqual(otherSession?.revocationOnly, false)
    let otherBytes = try await vault.read("originals/other-account", accountId: other.account.id)
    XCTAssertEqual(otherBytes, Data([2]))
    let removed = try await vault.url("originals/kept-until-canceled", accountId: state.account.id)
    XCTAssertFalse(FileManager.default.fileExists(atPath: removed.path))
  }
}

extension AppModelTests {
  func testCancelScheduledPublicationSignsOutOnlyAfterConfirmedCancellation() async throws {
    for status in [PublicationStatus.canceled, .processing, .published] {
      var state = workspaceWithPhotos(remote: true)
      var intent = try PublicationIntent.review(
        voyage: state.voyages[0], photos: state.photos, at: Date())
      intent.serverId = UUID()
      intent.status = .processing
      state.publicationIntents = [intent]
      var publish = OutboxOperation(
        accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
        kind: .publish(intentId: intent.id))
      publish.state = .applied
      state.operations = [publish]
      let store = try CoreDataWorkspaceStore(url: nil)
      try await store.insert(state)
      let credentials = TestCredentials()
      await credentials.save(
        SessionCredential(accountId: state.account.id, token: "synthetic-cancel-before-signout"))
      let model = AppModel(
        store: store,
        client: APIClient(
          origin: try testOrigin(), transport: StubHTTP([HTTPResponse(status: 204, body: Data())]),
          credentials: credentials), credentials: credentials)
      await model.restore(accountId: state.account.id)
      model.capabilities = try APIJSON.decoder().decode(
        ClientCapabilities.self,
        from: Data(
          contentsOf: Fixtures.directory("api").appendingPathComponent("native_capabilities.json")))
      let remote = StubSyncRemote()
      let receipt = OperationReceipt.publication(
        intentId: intent.serverId!, status: status, revision: "7",
        confirmedPublished: status == .published)
      await remote.setPublicationResult(receipt)
      await remote.setOutcomes([receipt])
      model.coordinators[state.account.id] = SyncCoordinator(store: store, remote: remote)
      let result = await model.cancelPublicationAndSignOut(intentIds: [intent.id])
      let expected: PublicationSignOutResult =
        status == .canceled
        ? .signedOut : status == .published ? .published([state.voyages[0].id]) : .waiting
      XCTAssertEqual(result, expected)
      XCTAssertEqual(model.activeAccountId, status == .canceled ? nil : state.account.id)
      if status != .canceled { XCTAssertTrue(model.signedIn) }
      if status == .processing { XCTAssertNotNil(model.error) }
      if status == .published {
        XCTAssertTrue(model.workspace?.voyages[0].confirmedPublished == true)
        await remote.setOutcomes([
          .applied(targetId: state.voyages[0].serverId!, revision: "8", previousRevision: "7")
        ])
        let signedOut = await model.unpublishAndSignOut(voyageIds: [state.voyages[0].id])
        XCTAssertTrue(signedOut)
        let privateVoyage = try await store.read(accountId: state.account.id).voyages[0]
        XCTAssertFalse(privateVoyage.confirmedPublished)
      }
    }
  }

  func testOfflinePublicationCancellationDoesNotSignOutOrRevokeItsCredential() async throws {
    let state = workspaceWithPhotos(remote: true)
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(
      SessionCredential(accountId: state.account.id, token: "stay-usable-until-choice"))
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: state.account.id)
    model.changeNetwork(.offline)
    let result = await model.cancelPublicationAndSignOut(intentIds: [UUID()])
    XCTAssertEqual(result, .waiting)
    XCTAssertEqual(model.activeAccountId, state.account.id)
    let credential = await credentials.read(accountId: state.account.id)
    XCTAssertEqual(credential?.revocationOnly, false)
  }
}

extension AppModelTests {
  func testUnpublishBeforeSignOutKeepsTheSessionWhenTheWebsiteRejectsTheChange() async throws {
    var state = workspaceWithPhotos(remote: true)
    state.voyages[0].confirmedPublished = true
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    await credentials.save(
      SessionCredential(accountId: state.account.id, token: "keep-session-for-resolution"))
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: state.account.id)
    model.capabilities = try APIJSON.decoder().decode(
      ClientCapabilities.self,
      from: Data(
        contentsOf: Fixtures.directory("api").appendingPathComponent("native_capabilities.json")))
    let remote = StubSyncRemote()
    await remote.setOutcomes([.rejected("Review the website revision", status: 412)])
    model.coordinators[state.account.id] = SyncCoordinator(store: store, remote: remote)
    let signedOut = await model.unpublishAndSignOut(voyageIds: [state.voyages[0].id])
    XCTAssertFalse(signedOut)
    XCTAssertTrue(model.signedIn)
    XCTAssertNotNil(model.error)
    XCTAssertTrue(model.workspace?.voyages[0].confirmedPublished == true)
    let credential = await credentials.read(accountId: state.account.id)
    XCTAssertEqual(credential?.revocationOnly, false)
  }
}
