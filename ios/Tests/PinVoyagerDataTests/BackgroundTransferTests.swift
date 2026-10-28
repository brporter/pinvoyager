import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

actor StubTransferDriver: TransferDriver {
  var active: [SystemTransfer] = []
  var resumed: [Int] = []
  var creations = 0
  var creationFails = false
  var canceled: [String] = []
  var configured: [String: SyncPolicy] = [:]
  var pending: [TransferCompletion] = []
  var creationHook: (@Sendable () async -> Void)?
  func onCreation(_ hook: @escaping @Sendable () async -> Void) { creationHook = hook }
  func addActive(_ task: SystemTransfer) { active.append(task) }
  func saveCompletion(_ completion: TransferCompletion) { pending.append(completion) }
  func pendingCompletions(sessionIdentifier: String) -> [TransferCompletion] {
    pending.filter { $0.descriptor.sessionIdentifier == sessionIdentifier }
  }
  func discardCompletion(_ completion: TransferCompletion) {
    pending.removeAll { $0.descriptor == completion.descriptor && $0.taskId == completion.taskId }
  }
  func configure(sessionIdentifier: String, policy: SyncPolicy) {
    configured[sessionIdentifier] = policy
  }
  func tasks(sessionIdentifier: String) -> [SystemTransfer] {
    active.filter { $0.descriptor.sessionIdentifier == sessionIdentifier }
  }
  func createSuspended(
    sessionIdentifier: String, request: URLRequest, body: URL, descriptor: TransferDescriptor,
    policy: SyncPolicy
  ) async throws -> SystemTransfer {
    if creationFails { throw URLError(.backgroundSessionInUseByAnotherProcess) }
    creations += 1
    let task = SystemTransfer(id: creations, descriptor: descriptor)
    active.append(task)
    await creationHook?()
    return task
  }
  func resume(_ task: SystemTransfer, sessionIdentifier: String) { resumed.append(task.id) }
  func cancel(_ task: SystemTransfer, sessionIdentifier: String) { active.removeAll { $0 == task } }
  func cancel(sessionIdentifier: String) {
    active.removeAll { $0.descriptor.sessionIdentifier == sessionIdentifier }
    canceled.append(sessionIdentifier)
  }
  func setCreationFailure() { creationFails = true }
}

private struct UploadFixture {
  let workspace: Workspace
  let store: CoreDataWorkspaceStore
  let vault: FileVault
  let operation: OutboxOperation
  let request: MutationRequest
  let service: BackgroundUploadService
}

private func uploadFixture(
  directory: URL, driver: StubTransferDriver, cellularException: Bool = false
) async throws -> UploadFixture {
  let vault = try FileVault(root: directory)
  let store = try CoreDataWorkspaceStore(url: nil)
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  state.photos[0].cellularException = cellularException ? true : nil
  var operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .upload(photoId: state.photos[0].id, publicConsent: false))
  let request = try RequestFactory.prepare(operation, workspace: state, now: Date())
  operation.request = request
  operation.state = .submitted
  state.operations = [operation]
  try await store.insert(state)
  try await vault.write(
    Data([0xff, 0xd8, 0xff, 1]), relativePath: state.photos[0].source!.relativePath,
    accountId: state.account.id)
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: state.account.id, token: "token"))
  let service = BackgroundUploadService(
    store: store, vault: vault, credentials: credentials, driver: driver, origin: try testOrigin(),
    maxBytes: 1000)
  return UploadFixture(
    workspace: state, store: store, vault: vault, operation: operation,
    request: request, service: service)
}

@Test func backgroundCompletionIsUncertainAndOldGenerationCannotMutateNewSession() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  let state = fixture.workspace
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request, workspace: state)
  let descriptor = TransferDescriptor(
    accountId: state.account.id, generation: 0, operationId: fixture.operation.id)
  try await fixture.service.completed(descriptor)
  #expect(
    try await fixture.store.read(accountId: state.account.id).operations[0].state == .uncertain)
  await fixture.service.cancel(accountId: state.account.id)
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request, workspace: state)
  // Reuse the immutable body for a new system task after cancellation.
  #expect(await driver.creations == 2)
  let transfer = try #require(
    try await fixture.store.read(accountId: state.account.id).transfers.first)
  #expect(transfer.id == fixture.operation.id)
  let before = try await fixture.vault.read(transfer.bodyPath, accountId: state.account.id)
  #expect(!before.isEmpty)
  try await LocalRepository(store: fixture.store).lock(accountId: state.account.id)
  try await fixture.service.completed(descriptor)
  #expect(try await fixture.store.read(accountId: state.account.id).account.generation == 1)
  await #expect(throws: AuthenticationError.self) {
    try await fixture.service.submit(
      operation: fixture.operation, request: fixture.request,
      workspace: fixture.store.read(accountId: state.account.id))
  }
}

@Test func taskCreationFailurePreservesBodyAndSourceForRecovery() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  await driver.setCreationFailure()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  await #expect(throws: URLError.self) {
    try await fixture.service.submit(
      operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  }
  let state = try await fixture.store.read(accountId: fixture.workspace.account.id)
  #expect(state.transfers.count == 1)
  #expect(state.transfers[0].taskId == nil)
  #expect(
    try await fixture.vault.read(
      fixture.workspace.photos[0].source!.relativePath, accountId: state.account.id
    ).count == 4)
}

@Test func transferDescriptorRejectsMalformedAndPreservesAccountGeneration() {
  let descriptor = TransferDescriptor(accountId: UUID(), generation: 9, operationId: UUID())
  #expect(TransferDescriptor(rawValue: descriptor.rawValue) == descriptor)
  for value in ["", "other:9:id", "\(UUID()):-1:\(UUID())", "\(UUID()):no:\(UUID())"] {
    #expect(TransferDescriptor(rawValue: value) == nil)
  }
}

@Test func backgroundSchedulePersistsBodyAndAdoptsExistingTask() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory)
  let store = try CoreDataWorkspaceStore(url: nil)
  var workspace = unlockedWorkspace()
  var voyage = LocalVoyage(
    ownerId: workspace.account.id, fields: VoyageFields(title: "x", description: "y"))
  voyage.serverId = UUID()
  voyage.revision = "1"
  workspace.voyages.append(voyage)
  var photo = LocalPhoto(voyageId: voyage.id, filename: "one.jpg")
  photo.source = SourceFile(
    relativePath: "originals/test", contentHash: "abc", byteCount: 4, mediaType: "image/jpeg")
  photo.transfer = .saved
  workspace.photos.append(photo)
  let operation = OutboxOperation(
    accountId: workspace.account.id, generation: 0, voyageId: voyage.id,
    kind: .upload(photoId: photo.id, publicConsent: false))
  workspace.operations.append(operation)
  try await store.insert(workspace)
  try await vault.write(
    Data([0xff, 0xd8, 0xff, 1]), relativePath: "originals/test", accountId: workspace.account.id)
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: workspace.account.id, token: "token"))
  let driver = StubTransferDriver()
  let service = BackgroundUploadService(
    store: store, vault: vault, credentials: credentials, driver: driver, origin: try testOrigin(),
    maxBytes: 1_000)
  let request = try RequestFactory.prepare(operation, workspace: workspace, now: Date())
  try await service.submit(operation: operation, request: request, workspace: workspace)
  try await service.submit(operation: operation, request: request, workspace: workspace)
  #expect(await driver.creations == 1)
  #expect(await driver.resumed == [1, 1])
  let saved = try await store.read(accountId: workspace.account.id)
  #expect(saved.transfers.count == 1)
  #expect(saved.transfers.first?.taskId == 1)
  #expect(saved.photos.first?.transfer == .scheduled)
}

@Test func backgroundRecoveryRestoresSavedPoliciesAndCancelsStaleGenerations() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  let account = fixture.workspace.account.id
  let identifier = TransferDescriptor(
    accountId: account, generation: 0, operationId: fixture.operation.id
  ).sessionIdentifier
  try await fixture.store.update(accountId: account) { $0.policy.wifiOnly = false }
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let recovery = BackgroundTransferRecovery(
    store: fixture.store, credentials: credentials, driver: driver)
  try await recovery.restore()
  #expect(await driver.configured[identifier]?.wifiOnly == false)
  #expect(await driver.canceled.isEmpty)
  try await fixture.store.update(accountId: account) { $0.account.generation += 1 }
  try await recovery.restore()
  #expect(await driver.canceled == [identifier])
}

@Test func transferSessionsRequireExactIdentifiersAndNonnegativeGenerations() {
  let account = UUID()
  let session = TransferSession(accountId: account, generation: 3)
  #expect(TransferSession(identifier: session.identifier) == session)
  for value in [
    "", "com.pinvoyager.upload.\(account)", "com.pinvoyager.upload.\(account).-1",
    "com.pinvoyager.upload.other.0", "other.pinvoyager.upload.\(account).0",
    session.identifier + ".extra",
  ] {
    #expect(TransferSession(identifier: value) == nil)
  }
}

@Test func backgroundResponsesPersistRetryClassificationAndRejectDuplicateCallbacks() async throws {
  for (status, expected) in [
    (201, OperationState.uncertain), (401, .uncertain), (403, .attention), (404, .conflict),
    (408, .uncertain), (412, .conflict), (413, .attention), (415, .attention), (422, .attention),
    (429, .uncertain), (503, .uncertain),
  ] {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let driver = StubTransferDriver()
    let fixture = try await uploadFixture(directory: directory, driver: driver)
    try await fixture.service.submit(
      operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
    let descriptor = TransferDescriptor(
      accountId: fixture.workspace.account.id, generation: 0, operationId: fixture.operation.id)
    let completion = TransferCompletion(
      descriptor: descriptor, taskId: 1,
      response: response(
        status, "{\"message\":\"Rejected safely\"}",
        headers: ["Retry-After": "120", "x-request-id": "upload-reference"]))
    let recovery = BackgroundTransferRecovery(
      store: fixture.store, credentials: TestCredentials(), driver: driver)
    let now = Date(timeIntervalSince1970: 1_800_000_000)
    try await recovery.completed(completion, now: now, randomUnit: 0.5)
    let saved = try await fixture.store.read(accountId: descriptor.accountId)
    #expect(saved.operations[0].state == expected)
    if [408, 429, 503].contains(status) {
      #expect(saved.operations[0].attempts == 1)
      #expect(saved.operations[0].nextAttempt >= now.addingTimeInterval(120))
      try await recovery.completed(completion, now: now, randomUnit: 0.5)
      #expect(
        try await fixture.store.read(accountId: descriptor.accountId).operations[0].attempts == 1)
    }
    if status == 401 {
      #expect(saved.account.access == .requiresSignIn)
      #expect(saved.account.generation == 1)
    }
    if status != 201 { #expect(saved.operations[0].requestId == "upload-reference") }
    #expect(saved.photos.first { $0.id == fixture.workspace.photos[0].id }?.source != nil)
  }
}

@Test func anOldTaskCompletionCannotChangeAReplacementTaskInTheSameSession() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  let account = fixture.workspace.account.id
  try await fixture.store.update(accountId: account) { $0.transfers[0].taskId = 2 }
  let descriptor = TransferDescriptor(
    accountId: account, generation: 0, operationId: fixture.operation.id)
  try await BackgroundTransferRecovery(
    store: fixture.store, credentials: TestCredentials(), driver: driver
  ).completed(TransferCompletion(descriptor: descriptor, taskId: 1, response: response(415, "{}")))
  let saved = try await fixture.store.read(accountId: account)
  #expect(saved.operations[0].state == .submitted)
  #expect(!saved.transfers[0].completed)
}

@Test func savedBackgroundFailuresReplayBeforeAnyTaskCanResume() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  let account = fixture.workspace.account.id
  let descriptor = TransferDescriptor(
    accountId: account, generation: 0, operationId: fixture.operation.id)
  await driver.saveCompletion(
    TransferCompletion(descriptor: descriptor, taskId: 1, response: response(415, "{}")))
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let recovery = BackgroundTransferRecovery(
    store: fixture.store, credentials: credentials, driver: driver)
  try await recovery.restore()
  let saved = try await fixture.store.read(accountId: account)
  #expect(saved.operations[0].state == .attention)
  #expect(saved.transfers[0].completed)
  #expect(await driver.pending.isEmpty)
  #expect(await driver.resumed == [1])
}

@Test func removingAPhotoWhileItsTaskIsCreatedCancelsOnlyThatTaskBeforeResume() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  let account = fixture.workspace.account.id
  let sibling = SystemTransfer(
    id: 999, descriptor: TransferDescriptor(accountId: account, generation: 0, operationId: UUID()))
  await driver.addActive(sibling)
  await driver.onCreation {
    try? await LocalRepository(store: fixture.store).removePhoto(
      accountId: account, photoId: fixture.workspace.photos[0].id)
  }
  await #expect(throws: WorkspaceError.self) {
    try await fixture.service.submit(
      operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  }
  #expect(await driver.resumed.isEmpty)
  #expect(await driver.active == [sibling])
  #expect(await driver.canceled.isEmpty)
  #expect(
    try await fixture.store.read(accountId: account).photos.first {
      $0.id == fixture.workspace.photos[0].id
    }?.source != nil)
}

@Test func cellularBackgroundSessionKeepsItsExceptionAcrossRecoveryAndCancelsOnRevocation()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(
    directory: directory, driver: driver, cellularException: true)
  let account = fixture.workspace.account.id
  let descriptor = TransferDescriptor(
    accountId: account, generation: 0,
    operationId: fixture.operation.id, cellularException: true)
  #expect(TransferDescriptor(rawValue: descriptor.rawValue) == descriptor)
  #expect(TransferSession(identifier: descriptor.sessionIdentifier)?.cellularException == true)
  #expect(TransferDescriptor(rawValue: descriptor.rawValue + ":") == nil)
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  #expect(await driver.configured[descriptor.sessionIdentifier]?.wifiOnly == false)
  #expect(await driver.configured[descriptor.sessionIdentifier]?.allowsConstrained == false)
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let recovery = BackgroundTransferRecovery(
    store: fixture.store, credentials: credentials, driver: driver)
  try await recovery.restore()
  #expect(await driver.resumed == [1, 1])
  let photoId = fixture.workspace.photos[0].id
  try await fixture.store.update(accountId: account) { state in
    state.photos[state.photos.firstIndex { $0.id == photoId }!].cellularException = nil
  }
  try await recovery.restore()
  #expect(await driver.active.isEmpty)
  try await fixture.store.update(accountId: account) { state in
    state.photos[state.photos.firstIndex { $0.id == photoId }!].cellularException = true
  }
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request,
    workspace: fixture.store.read(accountId: account))
  await fixture.service.cancel(accountId: account, generation: 0)
  #expect(await driver.active.isEmpty)
  #expect(await driver.canceled.contains(descriptor.sessionIdentifier))
}

@Test func removingAnUncertainUploadCancelsItsTaskWithoutReplayingBytesOrRequiringSignIn()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  let account = fixture.workspace.account.id
  try await fixture.service.submit(
    operation: fixture.operation, request: fixture.request,
    workspace: fixture.workspace)
  try await LocalRepository(store: fixture.store).removePhoto(
    accountId: account,
    photoId: fixture.workspace.photos[0].id)
  let remote = StubSyncRemote()
  await remote.setOutcomes([.removed(revision: "2", previousRevision: "1")])
  let coordinator = SyncCoordinator(store: fixture.store, remote: remote, uploads: fixture.service)
  try await coordinator.run(accountId: account, network: .wifi(constrained: false))
  let saved = try await fixture.store.read(accountId: account)
  #expect(saved.account.access == .unlocked)
  #expect(!saved.policy.paused)
  #expect(await driver.active.isEmpty)
  #expect(await driver.creations == 1)
  #expect(saved.operations.first { $0.id == fixture.operation.id }?.state == .canceled)
  #expect(saved.operations.first { $0.id == fixture.operation.id }?.request == fixture.request)
  #expect(saved.photos.first { $0.id == fixture.workspace.photos[0].id }?.transfer == .removed)
}

@Test func voyagePauseBetweenTaskCreationAndResumeCancelsOnlyThatTask() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let driver = StubTransferDriver()
  let fixture = try await uploadFixture(directory: directory, driver: driver)
  let other = SystemTransfer(
    id: 999,
    descriptor: TransferDescriptor(
      accountId: fixture.workspace.account.id,
      generation: 0, operationId: UUID()))
  await driver.addActive(other)
  await driver.onCreation {
    try? await LocalRepository(store: fixture.store).setUploadsPaused(
      accountId: fixture.workspace.account.id,
      voyageId: fixture.workspace.voyages[0].id, paused: true)
  }
  await #expect(throws: WorkspaceError.locked) {
    try await fixture.service.submit(
      operation: fixture.operation, request: fixture.request, workspace: fixture.workspace)
  }
  #expect(await driver.resumed.isEmpty)
  #expect(await driver.active == [other])
  #expect(
    try await fixture.store.read(accountId: fixture.workspace.account.id).photos.first {
      $0.id == fixture.workspace.photos[0].id
    }?.source != nil)
}
