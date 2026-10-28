import PinVoyagerData
import PinVoyagerDomain
import XCTest

@testable import PinVoyager

@MainActor final class DevicePreferencesModelTests: XCTestCase {
  func testSignedOutSettingsPersistAndOnlyInitializeANewAccountWorkspace() async throws {
    let suite = "pinvoyager.model.preferences.\(UUID())"
    defer { UserDefaults.standard.removePersistentDomain(forName: suite) }
    let preferences = UserDefaultsDevicePreferences(suiteName: suite)
    let store = try CoreDataWorkspaceStore(url: nil)
    let credentials = TestCredentials()
    let client = APIClient(
      origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials)
    let first = AppModel(
      store: store, client: client, credentials: credentials, devicePreferences: preferences)
    await first.restore(accountId: nil)
    var policy = SyncPolicy()
    policy.wifiOnly = false
    policy.allowsConstrained = true
    await first.setSyncPolicy(policy)
    let reopened = AppModel(
      store: store, client: client, credentials: credentials, devicePreferences: preferences)
    await reopened.restore(accountId: nil)
    XCTAssertEqual(reopened.guestPolicy, policy)
    XCTAssertNil(reopened.workspace)
    let accountIds = try await store.accountIds()
    XCTAssertTrue(accountIds.isEmpty)
    let session = try APIJSON.decoder().decode(
      SessionExchange.self,
      from: Data(
        contentsOf: Fixtures.directory("api").appendingPathComponent("native_session_exchange.json")
      ))
    try await reopened.acceptSession(session)
    XCTAssertEqual(reopened.workspace?.policy.wifiOnly, false)
    var accountPolicy = policy
    accountPolicy.wifiOnly = true
    await reopened.setSyncPolicy(accountPolicy)
    let rememberedDevicePolicy = try await preferences.load()
    XCTAssertEqual(rememberedDevicePolicy, policy)
    let another = AppModel(
      store: store, client: client, credentials: credentials, devicePreferences: preferences)
    await another.restore(accountId: session.user.id)
    XCTAssertEqual(another.workspace?.policy.wifiOnly, true)
    XCTAssertEqual(another.guestPolicy.wifiOnly, false)
  }
}

private actor FailedDevicePreferences: DevicePreferences {
  func load() throws -> SyncPolicy { throw CocoaError(.fileReadCorruptFile) }
  func save(_ policy: SyncPolicy) throws { throw CocoaError(.fileWriteNoPermission) }
}

extension DevicePreferencesModelTests {
  func testDamagedDeviceSettingsUseConservativeDefaultsAndFailedSavesStayVisible() async throws {
    let store = try CoreDataWorkspaceStore(url: nil)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials, devicePreferences: FailedDevicePreferences())
    await model.restore(accountId: nil)
    XCTAssertTrue(model.guestPolicy.wifiOnly)
    XCTAssertNotNil(model.error)
    var policy = SyncPolicy()
    policy.wifiOnly = false
    await model.setSyncPolicy(policy)
    XCTAssertTrue(model.guestPolicy.wifiOnly)
    XCTAssertNotNil(model.error)
  }
}

extension DevicePreferencesModelTests {
  func testRevokingBatchCellularPermissionCancelsPersistedSessionsWhileOffline() async throws {
    let state = workspaceWithPhotos()
    let account = state.account.id
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([]), credentials: credentials), credentials: credentials)
    await model.restore(accountId: account)
    let ids = try await model.repository.beginImports(
      accountId: account,
      voyageId: state.voyages[0].id, filenames: ["Cellular.jpg"], cellularException: true)
    await model.reloadWorkspace()
    let batch = try XCTUnwrap(model.workspace?.photos.first { ids.contains($0.id) }?.importBatchId)
    var canceled: [UUID] = []
    model.cancelPersistedTransfers = { canceled.append($0) }
    model.changeNetwork(.offline)
    await model.setCellularException(batchId: batch, allowed: false)
    XCTAssertEqual(canceled, [account])
    XCTAssertTrue(model.workspace?.policy.wifiOnly == true)
    XCTAssertNil(model.workspace?.photos.first { ids.contains($0.id) }?.cellularException)
  }
}

extension DevicePreferencesModelTests {
  func testLocalRemovalCancelsSavedUploadTasksEvenWhenNoCoordinatorCanRunOffline() async throws {
    let state = workspaceWithPhotos()
    let account = state.account.id
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([]), credentials: credentials), credentials: credentials)
    await model.restore(accountId: account)
    model.changeNetwork(.offline)
    var canceled: [UUID] = []
    model.cancelIneligibleTransfers = { canceled.append($0) }
    await model.perform {
      try await model.repository.removePhoto(accountId: account, photoId: state.photos[0].id)
    }
    XCTAssertEqual(canceled, [account])
    XCTAssertTrue(model.workspace?.photos.first { $0.id == state.photos[0].id }?.deleted == true)
  }
}

extension DevicePreferencesModelTests {
  func testIndividualPreferencesPreserveOtherChoicesAndIgnoreAnOldAccountWindow() async throws {
    let first = unlockedWorkspace()
    let second = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(first)
    try await store.insert(second)
    let credentials = TestCredentials()
    let model = AppModel(
      store: store,
      client: APIClient(
        origin: try testOrigin(), transport: StubHTTP([]), credentials: credentials),
      credentials: credentials)
    await model.restore(accountId: first.account.id)
    model.changeNetwork(.offline)
    await model.setPreference(.keepOriginals, value: false, accountId: first.account.id)
    await model.setPreference(.wifiOnly, value: false, accountId: first.account.id)
    XCTAssertFalse(model.workspace!.policy.keepOriginals)
    XCTAssertFalse(model.workspace!.policy.wifiOnly)
    await model.restore(accountId: second.account.id)
    await model.setPreference(.paused, value: true, accountId: first.account.id)
    XCTAssertFalse(model.workspace!.policy.paused)
    await model.signOut()
    await model.setPreference(.allowsConstrained, value: true, accountId: first.account.id)
    XCTAssertFalse(model.guestPolicy.allowsConstrained)
  }
}
