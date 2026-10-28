import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

func workspaceWithPhotos(remote: Bool = false) -> Workspace {
  var workspace = unlockedWorkspace()
  var voyage = LocalVoyage(
    ownerId: workspace.account.id, fields: VoyageFields(title: "Winter", description: "On foot"))
  if remote {
    voyage.serverId = UUID()
    voyage.revision = "7"
    voyage.serverFields = voyage.fields
  }
  workspace.voyages.append(voyage)
  for index in 0..<3 {
    var photo = LocalPhoto(voyageId: voyage.id, filename: "\(index).jpg")
    photo.source = SourceFile(
      relativePath: "originals/\(photo.id)", contentHash: String(repeating: "a", count: 64),
      byteCount: 10, mediaType: "image/jpeg")
    photo.order = index
    photo.transfer = .saved
    if remote {
      photo.serverId = UUID()
      photo.transfer = .accepted
      photo.variant = .ready
    }
    workspace.photos.append(photo)
  }
  return workspace
}

@Test func photoOrderingAndRemovalSurviveRelaunchWithoutPublishing() async throws {
  let state = workspaceWithPhotos()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  let voyageId = state.voyages[0].id
  let order = state.photos.reversed().map(\.id)
  try await repo.reorder(accountId: state.account.id, voyageId: voyageId, photoIds: order)
  #expect(
    try await store.read(accountId: state.account.id).photos(for: voyageId).map(\.id) == order)
  await #expect(throws: WorkspaceError.self) {
    try await repo.reorder(
      accountId: state.account.id, voyageId: voyageId, photoIds: [order[0], order[0]])
  }
  try await repo.removePhoto(accountId: state.account.id, photoId: order[0])
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos(for: voyageId).count == 2)
  #expect(saved.photos.first { $0.id == order[0] }?.source != nil)
  #expect(!saved.voyages[0].confirmedPublished)
}

@Test func livePublicUploadsRequireExplicitConsent() async throws {
  var state = workspaceWithPhotos(remote: true)
  state.voyages[0].confirmedPublished = true
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  var photo = LocalPhoto(voyageId: state.voyages[0].id, filename: "New.jpg")
  await #expect(throws: ValidationError.self) {
    try await repo.addPhoto(accountId: state.account.id, photo: photo)
  }
  photo.source = state.photos[0].source
  await #expect(throws: ValidationError.self) {
    try await repo.addPhoto(accountId: state.account.id, photo: photo)
  }
  try await repo.addPhoto(accountId: state.account.id, photo: photo, publicConsent: true)
  try await repo.addPhoto(accountId: state.account.id, photo: photo, publicConsent: true)
  #expect(try await store.read(accountId: state.account.id).photos.count == 4)
  try await repo.removePhoto(accountId: state.account.id, photoId: photo.id)
  #expect(
    try await store.read(accountId: state.account.id).operations.contains {
      if case .upload = $0.kind { $0.state == .canceled } else { false }
    })
}

@Test func localPublicationCanBeCanceledOrInvalidatedByEdit() async throws {
  let state = workspaceWithPhotos()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  let voyageId = state.voyages[0].id
  _ = try await repo.queuePublication(accountId: state.account.id, voyageId: voyageId)
  var saved = try await store.read(accountId: state.account.id)
  #expect(saved.publicationIntents.count == 1)
  try await repo.cancelPublication(
    accountId: state.account.id, intentId: saved.publicationIntents[0].id)
  #expect(
    try await store.read(accountId: state.account.id).publicationIntents[0].status == .canceled)
  _ = try await repo.queuePublication(accountId: state.account.id, voyageId: voyageId)
  try await repo.editVoyage(
    accountId: state.account.id, voyageId: voyageId,
    fields: VoyageFields(title: "Changed", description: "New"))
  saved = try await store.read(accountId: state.account.id)
  #expect(saved.publicationIntents.contains { $0.status == .needsReview })
  #expect(!saved.voyages[0].confirmedPublished)
}

@Test func armedPublicationRequiresServerCancellationBeforeEdits() async throws {
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.serverId = UUID()
  intent.status = .processing
  state.publicationIntents.append(intent)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  await #expect(throws: WorkspaceError.self) {
    try await repo.editVoyage(
      accountId: state.account.id, voyageId: state.voyages[0].id,
      fields: VoyageFields(title: "Changed", description: "New"))
  }
  try await repo.saveWorkingCopy(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    fields: VoyageFields(title: "Later", description: "Separate"))
  try await repo.cancelPublication(accountId: state.account.id, intentId: intent.id)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.publicationIntents[0].cancellationPending)
  #expect(saved.publicationIntents[0].isArmed)
  #expect(saved.workingCopies[0].fields.title == "Later")
  #expect(saved.voyages[0].fields.title == "Winter")
}

@Test func blockAndReportKeepPendingStateUntilAcknowledgment() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  let target = UUID()
  await #expect(throws: ValidationError.self) {
    try await repo.setBlocked(accountId: state.account.id, userId: state.account.id, blocked: true)
  }
  try await repo.setBlocked(accountId: state.account.id, userId: target, blocked: true)
  try await repo.setBlocked(accountId: state.account.id, userId: target, blocked: false)
  #expect(try await store.read(accountId: state.account.id).blockedAccounts.contains(target))
  await #expect(throws: ValidationError.self) {
    try await repo.report(
      accountId: state.account.id, target: "voyage:\(UUID())", reason: "spam",
      details: String(repeating: "x", count: 2_001))
  }
  try await repo.report(
    accountId: state.account.id, target: "voyage:\(UUID())", reason: "spam", details: "Reason")
  #expect(try await store.read(accountId: state.account.id).operations.count == 3)
}

@Test func deletingPrivateLocalVoyageNeverCreatesItRemotely() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  let voyageId = try await repo.createVoyage(
    accountId: state.account.id, fields: VoyageFields(title: "x", description: "y"))
  try await repo.deleteVoyage(accountId: state.account.id, voyageId: voyageId)
  let remote = StubSyncRemote()
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.isEmpty)
  #expect(try await store.read(accountId: state.account.id).voyages[0].deleted)
  #expect(try await store.read(accountId: state.account.id).operations.last?.state == .applied)
}

@Test func submittedCreationIsImmutableWhenLaterEditsAreSaved() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repo = LocalRepository(store: store)
  let id = try await repo.createVoyage(
    accountId: state.account.id, fields: VoyageFields(title: "Before", description: "One"))
  try await store.update(accountId: state.account.id) { state in
    state.operations[0].request = MutationRequest(
      method: "POST", path: "/api/voyages", body: Data("original".utf8))
    state.operations[0].state = .uncertain
  }
  try await repo.editVoyage(
    accountId: state.account.id, voyageId: id,
    fields: VoyageFields(title: "After", description: "Two"))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations.count == 2)
  #expect(saved.operations[0].request?.body == Data("original".utf8))
  #expect(saved.operations[1].dependencies == [saved.operations[0].id])
}

@Test func allMutationRequestsUseMappedIdentifiersAndRevisionPreconditions() throws {
  let state = workspaceWithPhotos(remote: true)
  let voyage = state.voyages[0]
  let kinds: [OperationKind] = [
    .edit(voyage.fields), .order(state.photos.map(\.id)), .unpublish, .deleteVoyage,
    .removePhoto(photoId: state.photos[0].id, uploadKey: state.photos[0].uploadKey),
  ]
  for kind in kinds {
    let operation = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: voyage.id, kind: kind,
      expectedRevision: "7")
    let request = try RequestFactory.prepare(operation, workspace: state, now: Date())
    #expect(request.path.contains(voyage.serverId!.uuidString))
    #expect(request.revision == "7")
  }
  for kind: OperationKind in [
    .block(userId: UUID(), blocked: true), .block(userId: UUID(), blocked: false),
    .report(target: "photo:id", reason: "other", details: "Text"),
  ] {
    let request = try RequestFactory.prepare(
      OutboxOperation(accountId: state.account.id, generation: 0, voyageId: nil, kind: kind),
      workspace: state, now: Date())
    #expect(request.path.hasPrefix("/api/"))
  }
}

@Test func publicationRequestsContainOnlyFrozenReviewedMembership() throws {
  var state = workspaceWithPhotos(remote: true)
  let now = Date()
  let intent = try PublicationIntent.review(voyage: state.voyages[0], photos: state.photos, at: now)
  state.publicationIntents.append(intent)
  let operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .publish(intentId: intent.id))
  let request = try RequestFactory.prepare(operation, workspace: state, now: now)
  let body = try #require(JSONSerialization.jsonObject(with: request.body!) as? [String: Any])
  #expect((body["photos"] as? [[String: String]])?.count == 3)
  #expect((body["metadata_digest"] as? String)?.count == 64)
  #expect(throws: ValidationError.self) {
    try RequestFactory.prepare(operation, workspace: state, now: now.addingTimeInterval(8 * 86_400))
  }
  state.publicationIntents[0].serverId = UUID()
  state.publicationIntents[0].status = .processing
  let cancellation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .cancelPublication(intentId: intent.id))
  #expect(try RequestFactory.prepare(cancellation, workspace: state, now: now).method == "DELETE")
}

@Test func explicitConflictResolutionKeepsOriginalRequestAndUsesNewRevision() async throws {
  var state = workspaceWithPhotos(remote: true)
  var conflicted = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .edit(VoyageFields(title: "Mine", description: "My text")), expectedRevision: "7")
  conflicted.state = .conflict
  conflicted.request = MutationRequest(
    method: "PATCH", path: "/saved", body: Data("immutable".utf8), revision: "7")
  state.operations.append(conflicted)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let website = VoyageFields(title: "Website", description: "Their text")
  let chosen = VoyageFields(title: "Mine", description: "Their text")
  try await LocalRepository(store: store).resolveMetadataConflict(
    accountId: state.account.id, operationId: conflicted.id, fields: chosen, websiteFields: website,
    revision: "8")
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages[0].fields == chosen)
  #expect(saved.voyages[0].revision == "8")
  #expect(saved.operations.first { $0.id == conflicted.id }?.request == conflicted.request)
  #expect(saved.operations.first { $0.id == conflicted.id }?.state == .canceled)
  #expect(
    saved.operations.contains {
      $0.expectedRevision == "8" && $0.request == nil && $0.state == .waiting
    })
  #expect(
    saved.photos.sorted { $0.id.uuidString < $1.id.uuidString }
      == state.photos.sorted { $0.id.uuidString < $1.id.uuidString })
}

@Test func recoveringDeletedDestinationUsesFreshPrivateIdentitiesAndRetainedSources() async throws {
  var state = workspaceWithPhotos(remote: true)
  state.photos[0].source = nil
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let id = try await LocalRepository(store: store).recoverAsNewDraft(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    sources: state.photos.filter { $0.source != nil }.map {
      RecoveredPhotoSource(original: $0, source: $0.source!, isOriginal: true)
    }, expectedGeneration: 0)
  let saved = try await store.read(accountId: state.account.id)
  let recovered = try #require(saved.voyages.first { $0.id == id })
  #expect(recovered.serverId == nil)
  #expect(!recovered.confirmedPublished)
  #expect(saved.photos(for: id).count == 2)
  #expect(
    Set(saved.photos(for: id).map(\.uploadKey)).isDisjoint(with: state.photos.map(\.uploadKey)))
  #expect(saved.operations.filter { $0.voyageId == id }.count == 3)
}

@Test func removingWebsitePhotosUsesTheirServerIdentityAndUncertainUploadsUseTheirKey() throws {
  var state = workspaceWithPhotos(remote: true)
  let photo = state.photos[0]
  let operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: photo.voyageId,
    kind: .removePhoto(photoId: photo.id, uploadKey: photo.uploadKey))
  let known = try RequestFactory.prepare(operation, workspace: state, now: Date())
  #expect(known.path == "/api/voyages/\(state.voyages[0].serverId!)/photos/\(photo.serverId!)")
  #expect(known.revision == "7")
  state.photos[0].serverId = nil
  state.voyages[0].revision = nil
  let uncertain = try RequestFactory.prepare(operation, workspace: state, now: Date())
  #expect(uncertain.path.hasSuffix("/uploads/\(photo.uploadKey)"))
  #expect(uncertain.revision == nil)
}

@Test func photoConflictResolutionUsesTheReviewedMembershipAndNeverResurrectsWebsiteDeletions()
  async throws
{
  for keepWebsite in [false, true] {
    var state = workspaceWithPhotos(remote: true)
    let voyage = state.voyages[0]
    var added = LocalPhoto(voyageId: voyage.id, filename: "My new photo")
    added.source = state.photos[0].source
    added.transfer = .saved
    added.order = 3
    state.photos.append(added)
    let upload = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: voyage.id,
      kind: .upload(photoId: added.id, publicConsent: false), expectedRevision: "7")
    var conflict = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: voyage.id,
      kind: .order(state.photos.prefix(3).map(\.id)), expectedRevision: "7")
    conflict.request = MutationRequest(method: "PUT", path: "/immutable-order", revision: "7")
    conflict.state = .conflict
    state.operations = [upload, conflict]
    let serverAdded = UUID()
    let serverIDs = [state.photos[2].serverId!, state.photos[0].serverId!, serverAdded]
    let remotePhotos = serverIDs.enumerated().map { position, id in
      RemotePhoto(
        id: id, position: position, capturedAt: nil, width: 2, height: 2, variantStatus: .ready,
        thumbnailUrl: "/thumb", displayUrl: "/display", fullUrl: "/full", coordinate: nil,
        placed: false)
    }
    let remote = RemoteVoyage(
      id: voyage.serverId!, ownerId: state.account.id, title: "Website name",
      description: "Website description", published: false, granularity: .km1,
      granularityLabel: "1 km",
      photoCount: 3, placedCount: 0, placeCount: 0, unplacedCount: 3, photos: remotePhotos,
      revision: "12")
    let metadata = serverIDs.map {
      OwnerPhoto(
        id: $0, width: 2, height: 2, capturedAt: nil, latitude: 10, longitude: 20,
        variantStatus: .ready)
    }
    let snapshot = OwnerVoyageSnapshot(voyage: remote, photos: metadata)
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let repository = LocalRepository(store: store)
    try await repository.resolvePhotoConflict(
      accountId: state.account.id, operationId: conflict.id,
      fields: remote.fields, snapshot: snapshot,
      choice: keepWebsite ? .website : .websiteAppendingLocal)
    let saved = try await store.read(accountId: state.account.id)
    let photos = saved.photos(for: voyage.id)
    #expect(photos.compactMap(\.serverId) == serverIDs)
    #expect(photos.count == (keepWebsite ? 3 : 4))
    if !keepWebsite { #expect(photos.last?.id == added.id) }
    #expect(saved.photos.first { $0.id == state.photos[1].id }?.deleted == true)
    #expect(saved.photos.first { $0.id == state.photos[1].id }?.source != nil)
    #expect(saved.operations.first { $0.id == conflict.id }?.request == conflict.request)
    #expect(saved.operations.first { $0.id == conflict.id }?.state == .canceled)
    #expect(saved.voyages[0].revision == "12")
    #expect(saved.voyages[0].serverFields == remote.fields)
    #expect(
      photos.first { $0.serverId == serverAdded }?.coordinate
        == Coordinate(latitude: 10, longitude: 20))
    #expect(
      saved.operations.first { $0.id == upload.id }?.state == (keepWebsite ? .canceled : .waiting))
    #expect(saved.photos.first { $0.id == added.id }?.source != nil)
  }
}

@Test func cancelingKnownPublicationNeverRequiresAMetadataRevision() throws {
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.serverId = UUID()
  intent.status = .processing
  state.publicationIntents = [intent]
  state.voyages[0].revision = nil
  let operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .cancelPublication(intentId: intent.id))
  let request = try RequestFactory.prepare(operation, workspace: state, now: Date())
  #expect(request.revision == nil)
  #expect(request.method == "DELETE")
}
