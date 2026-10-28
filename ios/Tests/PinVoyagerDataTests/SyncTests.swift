import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

actor StubSyncRemote: SyncRemote {
  var supported = true
  var submissions: [(UUID, MutationRequest)] = []
  var outcomes: [OperationReceipt] = []
  var receipts: [UUID: OperationReceipt] = [:]
  var unavailableReceipts: Set<UUID> = []
  func setUnavailableReceipts(_ ids: Set<UUID>) { unavailableReceipts = ids }
  var failSubmission = false
  var apiError: APIError?
  var transportError: URLError?
  var publicationResult: OperationReceipt = .inProgress
  var publicationError: APIError?
  var publicationQueries = 0
  var currentVisibility = false
  func setCurrentVisibility(_ value: Bool) { currentVisibility = value }
  func isPublished(voyageId: UUID) -> Bool { currentVisibility }
  func supportsNativeSync() -> Bool { supported }
  func receipt(operationId: UUID) throws -> OperationReceipt {
    if unavailableReceipts.contains(operationId) { throw URLError(.networkConnectionLost) }
    return receipts[operationId] ?? .notSeen
  }
  func submit(_ request: MutationRequest, operationId: UUID) throws -> OperationReceipt {
    submissions.append((operationId, request))
    if let apiError { throw apiError }
    if let transportError { throw transportError }
    if failSubmission { throw URLError(.networkConnectionLost) }
    return outcomes.isEmpty ? .notSeen : outcomes.removeFirst()
  }
  func setOutcomes(_ values: [OperationReceipt]) { outcomes = values }
  func setReceipt(_ receipt: OperationReceipt, id: UUID) { receipts[id] = receipt }
  func setFailure(_ value: Bool) { failSubmission = value }
  func setSupported(_ value: Bool) { supported = value }
  func setError(_ error: APIError?) { apiError = error }
  func setTransportError(_ error: URLError) { transportError = error }
  func publicationStatus(voyageId: UUID, intentId: UUID) throws -> OperationReceipt {
    publicationQueries += 1
    if let publicationError { throw publicationError }
    return publicationResult
  }
  func setPublicationError(_ error: APIError) { publicationError = error }
  func setPublicationResult(_ value: OperationReceipt) { publicationResult = value }
}

@Test func deletedCreationReceiptAndTLSFailureRequireAttentionWithoutRecreating() async throws {
  for deleted in [true, false] {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    _ = try await LocalRepository(store: store).createVoyage(
      accountId: state.account.id, fields: VoyageFields(title: "Trip", description: "Retained"))
    let remote = StubSyncRemote()
    if deleted {
      await remote.setOutcomes([.deleted])
    } else {
      await remote.setTransportError(URLError(.serverCertificateUntrusted))
    }
    let coordinator = SyncCoordinator(store: store, remote: remote)
    try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
    try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
    let saved = try await store.read(accountId: state.account.id)
    #expect(saved.operations[0].state == (deleted ? .conflict : .attention))
    #expect(saved.voyages[0].fields.description == "Retained")
    #expect(saved.voyages[0].serverId == nil)
    #expect(await remote.submissions.count == 1)
  }
}

@Test func rejectionClassesStopOrPauseWithoutDestroyingLocalWork() async throws {
  for (status, expected) in [
    (401, OperationState.uncertain), (403, .attention), (404, .conflict), (409, .attention),
    (412, .conflict), (422, .attention),
  ] {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    _ = try await LocalRepository(store: store).createVoyage(
      accountId: state.account.id, fields: VoyageFields(title: "Trip", description: "Saved"))
    let remote = StubSyncRemote()
    await remote.setError(APIError(response: response(status, "{\"message\":\"Rejected\"}")))
    try await SyncCoordinator(store: store, remote: remote).run(
      accountId: state.account.id, network: .wifi(constrained: false))
    let saved = try await store.read(accountId: state.account.id)
    #expect(saved.operations[0].state == expected)
    #expect(saved.voyages[0].fields.title == "Trip")
    #expect(saved.account.access == (status == 401 ? .requiresSignIn : .unlocked))
  }
}

@Test func transientFailureStopsAfterEightAttemptsAndExplicitRetryReconciles() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  _ = try await repo.createVoyage(
    accountId: state.account.id, fields: VoyageFields(title: "Trip", description: "Saved"))
  let remote = StubSyncRemote()
  await remote.setFailure(true)
  let coordinator = SyncCoordinator(store: store, remote: remote, randomUnit: { 0 })
  for _ in 0..<9 {
    try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  }
  let pending = try await store.read(accountId: state.account.id).operations[0]
  #expect(pending.attempts == 8)
  #expect(pending.state == .attention)
  #expect(await remote.submissions.count == 8)
  try await repo.retry(accountId: state.account.id, operationId: pending.id)
  let retry = try await store.read(accountId: state.account.id).operations[0]
  #expect(retry.attempts == 0)
  #expect(retry.state == .uncertain)
}

@Test func publicationIsOnlyConfirmedByTerminalServerStatus() async throws {
  var state = workspaceWithPhotos(remote: true)
  let localIntent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  state.publicationIntents.append(localIntent)
  state.operations.append(
    OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
      kind: .publish(intentId: localIntent.id)))
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let remote = StubSyncRemote()
  let serverIntentId = UUID()
  await remote.setOutcomes([
    .publication(intentId: serverIntentId, status: .processing, revision: "8")
  ])
  let coordinator = SyncCoordinator(store: store, remote: remote)
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  #expect(try await store.read(accountId: state.account.id).publicationsAreConfirmed == false)
  await remote.setPublicationResult(
    .publication(
      intentId: serverIntentId, status: .published, revision: "9", confirmedPublished: true))
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  #expect(try await store.read(accountId: state.account.id).voyages[0].confirmedPublished)
}

extension Workspace {
  fileprivate var publicationsAreConfirmed: Bool { voyages.contains(where: \.confirmedPublished) }
}

@Test func creationMapsRemoteIdentityAtomically() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  let repository = LocalRepository(store: store)
  let localId = try await repository.createVoyage(
    accountId: workspace.account.id, fields: VoyageFields(title: "Trip", description: "Winter"))
  let remote = StubSyncRemote()
  let remoteId = UUID()
  await remote.setOutcomes([.applied(targetId: remoteId, revision: "1")])
  let sync = SyncCoordinator(store: store, remote: remote)
  try await sync.run(accountId: workspace.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: workspace.account.id)
  #expect(saved.voyages.first?.id == localId)
  #expect(saved.voyages.first?.serverId == remoteId)
  #expect(saved.operations.first?.state == .applied)
  #expect(!saved.voyages.first!.confirmedPublished)
}

@Test func lostResponseUsesReceiptBeforeReplayingSameRequest() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  _ = try await LocalRepository(store: store).createVoyage(
    accountId: workspace.account.id, fields: VoyageFields(title: "Trip", description: "Winter"))
  let remote = StubSyncRemote()
  await remote.setFailure(true)
  let sync = SyncCoordinator(store: store, remote: remote, randomUnit: { 0 })
  try await sync.run(accountId: workspace.account.id, network: .wifi(constrained: false))
  let pending = try await store.read(accountId: workspace.account.id).operations.first!
  #expect(pending.state == .uncertain)
  await remote.setFailure(false)
  await remote.setReceipt(.applied(targetId: UUID(), revision: "1"), id: pending.id)
  try await sync.run(accountId: workspace.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.count == 1)
  #expect(try await store.read(accountId: workspace.account.id).operations.first?.state == .applied)
}

@Test func unsupportedOfflineAndLockedWorkNeverSubmits() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  let repository = LocalRepository(store: store)
  _ = try await repository.createVoyage(
    accountId: workspace.account.id, fields: VoyageFields(title: "Trip", description: "Winter"))
  let remote = StubSyncRemote()
  let sync = SyncCoordinator(store: store, remote: remote)
  try await sync.run(accountId: workspace.account.id, network: .offline)
  #expect(await remote.submissions.isEmpty)
  await remote.setSupported(false)
  await #expect(throws: WorkspaceError.self) {
    try await sync.run(accountId: workspace.account.id, network: .wifi(constrained: false))
  }
  try await repository.lock(accountId: workspace.account.id)
  await remote.setSupported(true)
  try await sync.run(accountId: workspace.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.isEmpty)
}

@Test func unpublicationLabelWaitsForServerConfirmation() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  var workspace = unlockedWorkspace()
  var voyage = LocalVoyage(
    ownerId: workspace.account.id, fields: VoyageFields(title: "Public", description: "Trip"))
  voyage.serverId = UUID()
  voyage.revision = "9"
  voyage.confirmedPublished = true
  workspace.voyages.append(voyage)
  try await store.insert(workspace)
  try await LocalRepository(store: store).unpublish(
    accountId: workspace.account.id, voyageId: voyage.id)
  #expect(
    try await store.read(accountId: workspace.account.id).voyages.first?.confirmedPublished == true)
  let remote = StubSyncRemote()
  await remote.setOutcomes([.applied(targetId: voyage.serverId!, revision: "10")])
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: workspace.account.id, network: .wifi(constrained: false))
  #expect(
    try await store.read(accountId: workspace.account.id).voyages.first?.confirmedPublished == false
  )
  let request = await remote.submissions.first!.1
  #expect(request.revision == "9")
  #expect(String(decoding: request.body!, as: UTF8.self).contains("false"))
}

@Test func orderingWaitsForUploadMappingsInsteadOfStoppingTheVoyage() async throws {
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  try await LocalRepository(store: store).reorder(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    photoIds: state.photos.reversed().map(\.id))
  let remote = StubSyncRemote()
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations[0].state == .waiting)
  #expect(await remote.submissions.isEmpty)
}

actor RecordedUpload: UploadSubmitting {
  var submitted: [UUID] = []
  func submit(operation: OutboxOperation, request: MutationRequest, workspace: Workspace) {
    submitted.append(operation.id)
  }
  func cancel(accountId: UUID) {}
}

@Test func uploadReceiptMapsPhotoBeforeCompleteOrdering() async throws {
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  state.photos = [state.photos[0]]
  let photo = state.photos[0]
  let upload = OutboxOperation(
    accountId: state.account.id, generation: 0,
    voyageId: photo.voyageId, kind: .upload(photoId: photo.id, publicConsent: false))
  state.operations = [upload]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  try await repo.reorder(
    accountId: state.account.id, voyageId: photo.voyageId, photoIds: [photo.id])
  let remote = StubSyncRemote()
  let uploads = RecordedUpload()
  let coordinator = SyncCoordinator(store: store, remote: remote, uploads: uploads)
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  #expect(await uploads.submitted == [upload.id])
  #expect(await remote.submissions.isEmpty)
  let photoId = UUID()
  await remote.setReceipt(.applied(targetId: photoId, revision: "2"), id: photo.uploadKey)
  await remote.setOutcomes([.applied(targetId: state.voyages[0].serverId!, revision: "3")])
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos[0].serverId == photoId)
  #expect(saved.photos[0].transfer == .accepted)
  #expect(saved.operations.allSatisfy { $0.state == .applied })
}

@Test func receiptInProgressAndRejectionDoNotBecomeAcceptance() async throws {
  for receipt: OperationReceipt in [.inProgress, .rejected("Correct this request")] {
    let state = unlockedWorkspace()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    _ = try await LocalRepository(store: store).createVoyage(
      accountId: state.account.id,
      fields: VoyageFields(title: "Trip", description: "Unconfirmed"))
    let remote = StubSyncRemote()
    await remote.setOutcomes([receipt])
    try await SyncCoordinator(store: store, remote: remote).run(
      accountId: state.account.id,
      network: .wifi(constrained: false))
    let saved = try await store.read(accountId: state.account.id)
    #expect(saved.operations[0].state == (receipt == .inProgress ? .uncertain : .attention))
    #expect(saved.voyages[0].serverId == nil)
    #expect(saved.operations[0].attempts == 0)
  }
}

@Test func acknowledgedDeletionAndUnblockFinishOnlyTheirIntendedEffects() async throws {
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  try await repo.removePhoto(accountId: state.account.id, photoId: state.photos[0].id)
  let remote = StubSyncRemote()
  await remote.setOutcomes([.deleted])
  let coordinator = SyncCoordinator(store: store, remote: remote)
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  let removed = try await store.read(accountId: state.account.id)
  #expect(removed.photos.first { $0.id == state.photos[0].id }?.transfer == .removed)
  #expect(removed.photos.first { $0.id == state.photos[0].id }?.source != nil)
  let blocked = UUID()
  try await repo.setBlocked(accountId: state.account.id, userId: blocked, blocked: true)
  try await repo.setBlocked(accountId: state.account.id, userId: blocked, blocked: false)
  await remote.setOutcomes([
    .applied(targetId: blocked, revision: "1"), .applied(targetId: blocked, revision: "2"),
  ])
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  #expect(try await store.read(accountId: state.account.id).blockedAccounts.isEmpty)
  #expect(try await store.read(accountId: state.account.id).voyages[0].confirmedPublished == false)
}

@Test func aRejectedReportDoesNotPreventBlockingAndBlockChangesRemainOrdered() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let target = UUID()
  try await repository.report(
    accountId: state.account.id, target: "voyage:\(UUID())", reason: "Spam", details: "Review")
  let report = try #require(try await store.read(accountId: state.account.id).operations.first)
  try await store.update(accountId: state.account.id) { state in
    state.operations[0].state = .attention
  }
  try await repository.setBlocked(accountId: state.account.id, userId: target, blocked: true)
  try await repository.setBlocked(accountId: state.account.id, userId: target, blocked: false)
  let queued = try await store.read(accountId: state.account.id)
  let block = try #require(
    queued.operations.first {
      if case .block(_, true) = $0.kind { return true }
      return false
    })
  let unblock = try #require(
    queued.operations.first {
      if case .block(_, false) = $0.kind { return true }
      return false
    })
  #expect(!block.dependencies.contains(report.id))
  #expect(unblock.dependencies.contains(block.id))
  let remote = StubSyncRemote()
  await remote.setOutcomes([
    .applied(targetId: target, revision: "0"), .applied(targetId: target, revision: "0"),
  ])
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations.first { $0.id == report.id }?.state == .attention)
  #expect(!saved.blockedAccounts.contains(target))
  #expect(await remote.submissions.count == 2)
}

@Test func acceptedUploadReceiptsReconcileOnCellularWhilePhotoTransfersWaitForWifi() async throws {
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  var operation = OutboxOperation(
    accountId: state.account.id, generation: state.account.generation,
    voyageId: state.voyages[0].id, kind: .upload(photoId: state.photos[0].id, publicConsent: false))
  operation.request = try RequestFactory.prepare(operation, workspace: state, now: Date())
  operation.state = .uncertain
  state.operations = [operation]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let remote = StubSyncRemote()
  let acceptedId = UUID()
  await remote.setReceipt(
    .applied(targetId: acceptedId, revision: "2"), id: state.photos[0].uploadKey)
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .cellular(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations[0].state == .applied)
  #expect(saved.photos.first { $0.id == state.photos[0].id }?.serverId == acceptedId)
  #expect(await remote.submissions.isEmpty)
}

@Test func confirmedPhotoDeletionAdvancesTheRevisionBeforeFollowingOrdering() async throws {
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  try await repository.removePhoto(accountId: state.account.id, photoId: state.photos[0].id)
  try await repository.reorder(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    photoIds: Array(state.photos.dropFirst().map(\.id)))
  let remote = StubSyncRemote()
  await remote.setOutcomes([
    .removed(revision: "8"), .applied(targetId: state.voyages[0].serverId!, revision: "9"),
  ])
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.count == 2)
  #expect(await remote.submissions.last?.1.revision == "8")
  #expect(try await store.read(accountId: state.account.id).voyages[0].revision == "9")
}

@Test func privacyCleanupDoesNotWaitForFailedOrWifiRestrictedUploads() async throws {
  for action in ["remove", "unpublish", "delete", "cancel"] {
    var state = workspaceWithPhotos()
    let voyage = state.voyages[0].id
    state.voyages[0].serverId = UUID()
    state.voyages[0].revision = "1"
    var upload = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: voyage,
      kind: .upload(photoId: state.photos[0].id, publicConsent: false))
    upload.request = try RequestFactory.prepare(upload, workspace: state, now: Date())
    upload.state = .attention
    state.operations = [upload]
    let intent = try PublicationIntent.review(
      voyage: state.voyages[0], photos: state.photos, at: Date())
    if action == "cancel" {
      var armed = intent
      armed.serverId = UUID()
      armed.status = .waitingForUploads
      state.publicationIntents = [armed]
    }
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let repository = LocalRepository(store: store)
    switch action {
    case "remove":
      try await repository.removePhoto(accountId: state.account.id, photoId: state.photos[0].id)
    case "unpublish": try await repository.unpublish(accountId: state.account.id, voyageId: voyage)
    case "delete": try await repository.deleteVoyage(accountId: state.account.id, voyageId: voyage)
    default:
      try await repository.cancelPublication(accountId: state.account.id, intentId: intent.id)
    }
    let remote = StubSyncRemote()
    let outcome: OperationReceipt
    switch action {
    case "cancel":
      outcome = .publication(
        intentId: state.publicationIntents[0].serverId!, status: .canceled, revision: "2")
    case "unpublish": outcome = .applied(targetId: state.voyages[0].serverId!, revision: "2")
    default: outcome = .removed(revision: "2")
    }
    await remote.setOutcomes([outcome])
    try await SyncCoordinator(store: store, remote: remote).run(
      accountId: state.account.id, network: .cellular(constrained: false))
    #expect(await remote.submissions.count == 1)
    #expect(
      try await store.read(accountId: state.account.id).operations.contains {
        $0.kind.priority == 0 && $0.state == .applied
      })
  }
}

@Test func publicationPollingUsesBackoffAndDoesNotStopUnrelatedVoyages() async throws {
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.serverId = UUID()
  intent.status = .processing
  state.publicationIntents = [intent]
  var publication = OutboxOperation(
    accountId: state.account.id, generation: 0,
    voyageId: state.voyages[0].id, kind: .publish(intentId: intent.id))
  publication.state = .applied
  state.operations = [publication]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  _ = try await LocalRepository(store: store).createVoyage(
    accountId: state.account.id,
    fields: VoyageFields(title: "Other voyage", description: "Must still synchronize"))
  let remote = StubSyncRemote()
  await remote.setPublicationError(
    APIError(response: response(503, "{}", headers: ["Retry-After": "120"])))
  await remote.setOutcomes([.applied(targetId: UUID(), revision: "1")])
  let now = Date()
  let coordinator = SyncCoordinator(store: store, remote: remote)
  try await coordinator.run(
    accountId: state.account.id, network: .wifi(constrained: false), now: now)
  try await coordinator.run(
    accountId: state.account.id, network: .wifi(constrained: false), now: now)
  let saved = try await store.read(accountId: state.account.id)
  let pending = try #require(saved.operations.first { $0.id == publication.id })
  #expect(pending.attempts == 1)
  #expect(pending.nextAttempt >= now.addingTimeInterval(120))
  #expect(await remote.publicationQueries == 1)
  #expect(await remote.submissions.count == 1)
}

@Test func cancellationOfAnUncertainPublicationRemainsQueuedUntilItsServerIdentityIsKnown()
  async throws
{
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let operationId = try await repository.queuePublication(
    accountId: state.account.id, voyageId: state.voyages[0].id)
  let remote = StubSyncRemote()
  await remote.setFailure(true)
  let coordinator = SyncCoordinator(store: store, remote: remote, randomUnit: { 0 })
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  let queued = try await store.read(accountId: state.account.id)
  let intent = try #require(queued.publicationIntents.first)
  try await repository.cancelPublication(accountId: state.account.id, intentId: intent.id)
  #expect(
    try await store.read(accountId: state.account.id).publicationIntents[0].cancellationPending)
  let serverId = UUID()
  await remote.setFailure(false)
  await remote.setReceipt(
    .publication(intentId: serverId, status: .processing, revision: "8"), id: operationId)
  await remote.setOutcomes([.publication(intentId: serverId, status: .canceled, revision: "8")])
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.publicationIntents[0].status == .canceled)
  #expect(!saved.publicationIntents[0].cancellationPending)
  #expect(await remote.submissions.last?.1.method == "DELETE")
}

@Test func historicalUnpublicationReceiptsUseCurrentVisibilityAndRetainTheirOriginalRevision()
  async throws
{
  var state = workspaceWithPhotos(remote: true)
  state.voyages[0].confirmedPublished = true
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  try await LocalRepository(store: store).unpublish(
    accountId: state.account.id, voyageId: state.voyages[0].id)
  let remote = StubSyncRemote()
  await remote.setCurrentVisibility(true)
  await remote.setOutcomes([.applied(targetId: state.voyages[0].serverId!, revision: "8")])
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages[0].confirmedPublished)
  #expect(saved.voyages[0].revision == "8")
  #expect(saved.operations[0].state == .applied)
}

@Test func aGenericVoyageReceiptCannotAcknowledgePublicationConsent() async throws {
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  _ = try await LocalRepository(store: store).queuePublication(
    accountId: state.account.id, voyageId: state.voyages[0].id)
  let remote = StubSyncRemote()
  await remote.setOutcomes([.applied(targetId: state.voyages[0].serverId!, revision: "8")])
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations[0].state == .uncertain)
  #expect(saved.publicationIntents[0].serverId == nil)
  #expect(!saved.voyages[0].confirmedPublished)
}

@Test func deletedDestinationsKeepMetadataAndPublicationChangesForExplicitRecovery() async throws {
  for publication in [false, true] {
    let state = workspaceWithPhotos(remote: true)
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let repository = LocalRepository(store: store)
    if publication {
      _ = try await repository.queuePublication(
        accountId: state.account.id, voyageId: state.voyages[0].id)
    } else {
      try await repository.editVoyage(
        accountId: state.account.id, voyageId: state.voyages[0].id,
        fields: VoyageFields(title: "Retained edits", description: "Saved locally"))
    }
    let remote = StubSyncRemote()
    await remote.setOutcomes([.deleted])
    try await SyncCoordinator(store: store, remote: remote).run(
      accountId: state.account.id, network: .wifi(constrained: false))
    let saved = try await store.read(accountId: state.account.id)
    #expect(saved.operations[0].state == .conflict)
    #expect(saved.photos.allSatisfy { $0.source != nil })
    if publication {
      #expect(saved.publicationIntents[0].status == .needsReview)
    } else {
      #expect(saved.voyages[0].fields.title == "Retained edits")
    }
  }
}

@Test func aLostRevisionRejectionStillOpensConflictRecoveryWithoutReplayingTheEdit() async throws {
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  try await LocalRepository(store: store).editVoyage(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    fields: VoyageFields(title: "My retained edit", description: "Saved before disconnect"))
  let remote = StubSyncRemote()
  await remote.setFailure(true)
  let sync = SyncCoordinator(store: store, remote: remote, randomUnit: { 0 })
  try await sync.run(accountId: state.account.id, network: .wifi(constrained: false))
  let operation = try #require(try await store.read(accountId: state.account.id).operations.first)
  await remote.setFailure(false)
  await remote.setReceipt(.rejected("The voyage changed.", status: 412), id: operation.id)
  try await sync.run(accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations[0].state == .conflict)
  #expect(saved.voyages[0].fields.title == "My retained edit")
  #expect(await remote.submissions.count == 1)
}

@Test func anUploadCannotRebaseAnEditAcrossAnUnseenWebsiteRevision() async throws {
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  var upload = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .upload(photoId: state.photos[0].id, publicConsent: false))
  upload.request = try RequestFactory.prepare(upload, workspace: state, now: Date())
  upload.state = .submitted
  state.operations = [upload]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  try await LocalRepository(store: store).editVoyage(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    fields: VoyageFields(title: "My offline edit", description: "Retained"))
  let remote = StubSyncRemote()
  await remote.setReceipt(
    .applied(targetId: UUID(), revision: "3", previousRevision: "2"), id: state.photos[0].uploadKey)
  await remote.setError(APIError(response: response(412, "{}")))
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.first?.1.revision == "1")
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages[0].revision == "1")
  #expect(
    saved.operations.first {
      if case .edit = $0.kind { return true }
      return false
    }?.state == .conflict)
}

@Test func publicationStatusDoesNotReplaceTheMetadataSnapshotRevision() async throws {
  var state = workspaceWithPhotos(remote: true)
  let baseline = state.voyages[0].revision
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
  let remote = StubSyncRemote()
  await remote.setPublicationResult(
    .publication(
      intentId: intent.serverId!, status: .published, revision: "99", confirmedPublished: true))
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages[0].confirmedPublished)
  #expect(saved.voyages[0].revision == baseline)
  #expect(saved.voyages[0].fields == state.voyages[0].fields)
}

@Test func disconnectedOwnRevisionEdgesSurviveRestartAndReconnectBeforeNewWrites() async throws {
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  for photo in state.photos.prefix(2) {
    var upload = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
      kind: .upload(photoId: photo.id, publicConsent: false))
    upload.request = try RequestFactory.prepare(upload, workspace: state, now: Date())
    upload.state = .submitted
    state.operations.append(upload)
  }
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("revision-edges.sqlite")
  let store = try CoreDataWorkspaceStore(url: url)
  try await store.insert(state)
  let remote = StubSyncRemote()
  await remote.setUnavailableReceipts([state.photos[0].uploadKey])
  await remote.setReceipt(
    .applied(targetId: UUID(), revision: "3", previousRevision: "2"), id: state.photos[1].uploadKey)
  try await SyncCoordinator(store: store, remote: remote, randomUnit: { 0 }).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  #expect(try await store.read(accountId: state.account.id).voyages[0].revision == "1")
  let reopened = try CoreDataWorkspaceStore(url: url)
  let fields = VoyageFields(title: "My next edit", description: "Saved after restart")
  try await LocalRepository(store: reopened).editVoyage(
    accountId: state.account.id, voyageId: state.voyages[0].id, fields: fields)
  await remote.setUnavailableReceipts([])
  await remote.setReceipt(
    .applied(targetId: UUID(), revision: "2", previousRevision: "1"), id: state.photos[0].uploadKey)
  await remote.setOutcomes([
    .applied(targetId: state.voyages[0].serverId!, revision: "4", previousRevision: "3")
  ])
  try await SyncCoordinator(store: reopened, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.first?.1.revision == "3")
  let saved = try await reopened.read(accountId: state.account.id)
  #expect(saved.voyages[0].revision == "4")
  #expect(saved.voyages[0].serverFields == fields)
  #expect(saved.operations.allSatisfy { $0.state == .applied })
}

@Test func receiptCompletionDatePersistsAndReplayDoesNotExtendActivityRetention() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let id = try await LocalRepository(store: store).createVoyage(
    accountId: state.account.id,
    fields: VoyageFields(title: "Completed draft", description: "A receipt was acknowledged"))
  let remote = StubSyncRemote()
  let serverId = UUID()
  await remote.setOutcomes([.applied(targetId: serverId, revision: "1", previousRevision: nil)])
  let coordinator = SyncCoordinator(store: store, remote: remote)
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  let applied = try #require(
    try await store.read(accountId: state.account.id).operations.first { $0.voyageId == id })
  #expect(applied.completedAt != nil)
  try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
  #expect(
    try await store.read(accountId: state.account.id).operations.first { $0.id == applied.id }?
      .completedAt == applied.completedAt)
}

@Test func preparingARequestCannotOverwriteAnEditOrDispatchAnOperationCanceledBeforeItsCheckpoint()
  async throws
{
  for cancel in [false, true] {
    let state = unlockedWorkspace()
    let base = try CoreDataWorkspaceStore(url: nil)
    try await base.insert(state)
    let repository = LocalRepository(store: base)
    let id = try await repository.createVoyage(
      accountId: state.account.id,
      fields: VoyageFields(title: "Earlier draft", description: "Before preparation"))
    let delayed = SuspendedUpdateStore(base)
    let remote = StubSyncRemote()
    let coordinator = SyncCoordinator(store: delayed, remote: remote)
    let sync = Task {
      try await coordinator.run(accountId: state.account.id, network: .wifi(constrained: false))
    }
    await delayed.waitUntilPaused()
    let edited = VoyageFields(
      title: "Edited during preparation", description: "Keep this latest value")
    if cancel {
      try await base.update(accountId: state.account.id) { $0.operations[0].state = .canceled }
    } else {
      try await repository.editVoyage(accountId: state.account.id, voyageId: id, fields: edited)
    }
    await delayed.continueUpdate()
    try await sync.value
    #expect(await remote.submissions.isEmpty)
    let saved = try await base.read(accountId: state.account.id)
    #expect(saved.operations[0].request == nil)
    #expect(saved.operations[0].state == (cancel ? .canceled : .waiting))
    if !cancel { #expect(saved.operations[0].kind == .create(edited)) }
  }
}
