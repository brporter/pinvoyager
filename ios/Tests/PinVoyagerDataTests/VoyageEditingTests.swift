import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func editorSessionsKeepLateDraftCallbacksInTheirOriginalAccountNamespace() async throws {
  let first = unlockedWorkspace()
  let second = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(first)
  try await store.insert(second)
  let repository = LocalRepository(store: store)
  let editor = VoyageEditSession(accountId: first.account.id, repository: repository)
  let fields = VoyageFields(title: "First account draft", description: "Private working copy")
  try await editor.saveWorkingCopy(fields)
  #expect(try await store.read(accountId: first.account.id).scratch == fields)
  #expect(try await store.read(accountId: second.account.id).scratch == nil)
  try await repository.lock(accountId: first.account.id)
  await #expect(throws: WorkspaceError.locked) { try await editor.saveWorkingCopy(fields) }
  await #expect(throws: WorkspaceError.locked) { try await editor.discardWorkingCopy() }
  #expect(try await store.read(accountId: second.account.id).scratch == nil)
}

@Test func anEditorCanSaveAnArmedVoyagesWorkingCopyWithoutChangingPublicationConsent() async throws
{
  var state = workspaceWithPhotos(remote: true)
  let voyage = state.voyages[0]
  var intent = try PublicationIntent.review(voyage: voyage, photos: state.photos, at: Date())
  intent.serverId = UUID()
  intent.status = .processing
  state.publicationIntents = [intent]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let editor = VoyageEditSession(
    accountId: state.account.id, voyageId: voyage.id, repository: LocalRepository(store: store))
  let changed = VoyageFields(title: "Keep these local edits", description: nil)
  try await editor.saveWorkingCopy(changed)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages[0].fields == voyage.fields)
  #expect(saved.workingCopies[0].fields == changed)
  #expect(saved.publicationIntents == [intent])
  #expect(saved.operations.isEmpty)
  await #expect(throws: WorkspaceError.publicationArmed) { try await editor.commit(changed) }
  try await editor.discardWorkingCopy()
  #expect(try await store.read(accountId: state.account.id).workingCopies.isEmpty)
}

@Test func editorCommitQueuesItsOwnVoyageAndInvalidFieldsRemainRecoverable() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let new = VoyageEditSession(accountId: state.account.id, repository: repository)
  try await new.saveWorkingCopy(VoyageFields(title: "Not ready", description: ""))
  await #expect(throws: ValidationError.self) { try await new.commit(VoyageFields()) }
  #expect(try await store.read(accountId: state.account.id).scratch != nil)
  let id = try await new.commit(VoyageFields(title: "Ready", description: "Saved locally"))
  let editor = VoyageEditSession(accountId: state.account.id, voyageId: id, repository: repository)
  let fields = VoyageFields(title: "Changed", description: nil)
  try await editor.saveWorkingCopy(fields)
  #expect(try await editor.commit(fields) == id)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages.first?.fields == fields)
  #expect(saved.scratch == nil)
  #expect(saved.workingCopies.isEmpty)
}

@Test func bulkPhotoRemovalIsAtomicAndKeepsUncertainUploadEvidenceAndSources() async throws {
  var state = workspaceWithPhotos(remote: true)
  let ids = state.photos.map(\.id)
  var upload = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .upload(photoId: ids[0], publicConsent: false))
  upload.state = .uncertain
  upload.request = MutationRequest(method: "POST", path: "/submitted-upload")
  state.operations = [upload]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let baseline = try await store.read(accountId: state.account.id)
  await #expect(throws: WorkspaceError.self) {
    try await repository.removePhotos(
      accountId: state.account.id, voyageId: state.voyages[0].id, photoIds: [ids[0], UUID()])
  }
  #expect(try await store.read(accountId: state.account.id) == baseline)
  try await repository.removePhotos(
    accountId: state.account.id, voyageId: state.voyages[0].id, photoIds: Set(ids))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos.allSatisfy { $0.deleted })
  #expect(
    Dictionary(uniqueKeysWithValues: saved.photos.map { ($0.id, $0.source) })
      == Dictionary(uniqueKeysWithValues: state.photos.map { ($0.id, $0.source) }))
  #expect(saved.operations.contains { $0.id == upload.id && $0.state == .uncertain })
  #expect(
    saved.operations.filter { if case .removePhoto = $0.kind { true } else { false } }.count
      == ids.count)
}

@Test func pausingOneVoyagePreservesOtherUploadsAndMetadataAndSurvivesReopening() async throws {
  var state = workspaceWithPhotos(remote: true)
  let other = LocalVoyage(
    ownerId: state.account.id,
    fields: VoyageFields(title: "Other voyage", description: "Independent"))
  state.voyages.append(other)
  let otherPhoto = LocalPhoto(voyageId: other.id, filename: "Other photo")
  state.photos.append(otherPhoto)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  try await repository.setUploadsPaused(
    accountId: state.account.id, voyageId: state.voyages[0].id, paused: true)
  let paused = try await store.read(accountId: state.account.id)
  #expect(paused.account.generation == state.account.generation)
  #expect(!paused.policy.paused)
  #expect(
    paused.uploadPolicy(for: state.photos[0].id).waitReason(
      network: .wifi(constrained: false), authenticated: true) == .paused)
  #expect(
    paused.uploadPolicy(for: otherPhoto.id).waitReason(
      network: .wifi(constrained: false), authenticated: true) == nil)
  try await repository.editVoyage(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    fields: VoyageFields(title: "Metadata still saves", description: nil))
  let encoded = try JSONEncoder().encode(try await store.read(accountId: state.account.id))
  #expect(
    try JSONDecoder().decode(Workspace.self, from: encoded).voyages.first {
      $0.id == state.voyages[0].id
    }?.uploadsPaused == true)
  try await repository.setUploadsPaused(
    accountId: state.account.id, voyageId: state.voyages[0].id, paused: false)
  #expect(
    try await store.read(accountId: state.account.id).uploadPolicy(for: state.photos[0].id).paused
      == false)
  try await repository.lock(accountId: state.account.id)
  await #expect(throws: WorkspaceError.locked) {
    try await repository.setUploadsPaused(
      accountId: state.account.id, voyageId: state.voyages[0].id, paused: true)
  }
}

@Test func bulkRemovalUndoIsAtomicAndStopsOnceAnyRemovalHasBeenSubmitted() async throws {
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let ids = Set(state.photos.prefix(2).map(\.id))
  try await repository.removePhotos(
    accountId: state.account.id, voyageId: state.voyages[0].id, photoIds: ids)
  try await repository.undoPhotoRemovals(
    accountId: state.account.id, voyageId: state.voyages[0].id, photoIds: ids)
  #expect(
    try await store.read(accountId: state.account.id).photos(for: state.voyages[0].id).count == 3)
  try await repository.removePhotos(
    accountId: state.account.id, voyageId: state.voyages[0].id, photoIds: ids)
  try await store.update(accountId: state.account.id) { current in
    let index = current.operations.lastIndex {
      if case .removePhoto = $0.kind { true } else { false }
    }!
    current.operations[index].request = MutationRequest(
      method: "DELETE", path: "/already-submitted")
    current.operations[index].state = .submitted
  }
  await #expect(throws: ValidationError.self) {
    try await repository.undoPhotoRemovals(
      accountId: state.account.id, voyageId: state.voyages[0].id, photoIds: ids)
  }
  #expect(
    try await store.read(accountId: state.account.id).photos(for: state.voyages[0].id).count == 1)
}

@Test func voyageDeletionUndoRestoresOnlyThePhotosAndOperationsActiveBeforeDeletion() async throws {
  var state = workspaceWithPhotos(remote: true)
  state.photos[0].deleted = true
  var edit = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .edit(state.voyages[0].fields))
  edit.state = .attention
  var completed = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id, kind: .order([]))
  completed.state = .applied
  state.operations = [edit, completed]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  try await repository.deleteVoyage(accountId: state.account.id, voyageId: state.voyages[0].id)
  try await repository.undoVoyageDeletion(
    accountId: state.account.id, voyageId: state.voyages[0].id)
  let restored = try await store.read(accountId: state.account.id)
  #expect(restored.voyages[0].deleted == false)
  #expect(restored.photos.first { $0.id == state.photos[0].id }?.deleted == true)
  #expect(restored.photos(for: state.voyages[0].id).count == 2)
  #expect(restored.operations.first { $0.id == edit.id }?.state == .attention)
  #expect(restored.operations.first { $0.id == completed.id }?.state == .applied)
  try await repository.deleteVoyage(accountId: state.account.id, voyageId: state.voyages[0].id)
  try await store.update(accountId: state.account.id) { current in
    let index = current.operations.lastIndex { $0.kind == .deleteVoyage }!
    current.operations[index].request = MutationRequest(method: "DELETE", path: "/submitted-delete")
    current.operations[index].state = .submitted
  }
  await #expect(throws: ValidationError.self) {
    try await repository.undoVoyageDeletion(
      accountId: state.account.id, voyageId: state.voyages[0].id)
  }
  #expect(try await store.read(accountId: state.account.id).voyages[0].deleted)
}

@Test func unpublicationNeverQueuesAnUnknownVoyageInAnotherAccountsWorkspace() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  await #expect(throws: WorkspaceError.missingVoyage) {
    try await LocalRepository(store: store).unpublish(accountId: state.account.id, voyageId: UUID())
  }
  #expect(try await store.read(accountId: state.account.id).operations.isEmpty)
}

@Test func anUnansweredPublicationRequestProtectsItsReviewedSnapshotUntilItsReceiptIsKnown()
  async throws
{
  var state = workspaceWithPhotos(remote: true)
  let voyage = state.voyages[0]
  let intent = try PublicationIntent.review(voyage: voyage, photos: state.photos, at: Date())
  var publish = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: voyage.id,
    kind: .publish(intentId: intent.id))
  publish.state = .uncertain
  publish.request = MutationRequest(method: "POST", path: "/possibly-armed-publication")
  state.publicationIntents = [intent]
  state.operations = [publish]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  await #expect(throws: WorkspaceError.publicationArmed) {
    try await repository.editVoyage(
      accountId: state.account.id, voyageId: voyage.id,
      fields: VoyageFields(title: "Keep this separate", description: "Unreviewed"))
  }
  await #expect(throws: WorkspaceError.publicationArmed) {
    try await repository.reorder(
      accountId: state.account.id, voyageId: voyage.id,
      photoIds: Array(state.photos.map(\.id).reversed()))
  }
  try await repository.saveWorkingCopy(
    accountId: state.account.id, voyageId: voyage.id,
    fields: VoyageFields(title: "Working copy", description: nil))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.voyages[0].fields == voyage.fields)
  #expect(saved.publicationIntents[0].fields == intent.fields)
  #expect(saved.operations.first { $0.id == publish.id }?.request == publish.request)
}

@Test func aDefinitivePublicationRejectionAllowsLocalCancellationOrAFreshReview() async throws {
  for cancel in [false, true] {
    var state = workspaceWithPhotos(remote: true)
    let voyage = state.voyages[0]
    let intent = try PublicationIntent.review(voyage: voyage, photos: state.photos, at: Date())
    var publish = OutboxOperation(
      accountId: state.account.id, generation: 0, voyageId: voyage.id,
      kind: .publish(intentId: intent.id))
    publish.request = MutationRequest(method: "POST", path: "/reviewed-publication")
    publish.state = .attention
    publish.nextAttempt = Date().addingTimeInterval(1000)
    state.publicationIntents = [intent]
    state.operations = [publish]
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let repository = LocalRepository(store: store)
    try await repository.cancelPublication(accountId: state.account.id, intentId: intent.id)
    let checking = try await store.read(accountId: state.account.id)
    #expect(checking.operations.first { $0.id == publish.id }?.state == .uncertain)
    #expect(checking.publicationMayProceed(intent))
    let remote = StubSyncRemote()
    await remote.setReceipt(
      .rejected("A fresh review is required", status: 422), id: publish.id)
    try await SyncCoordinator(store: store, remote: remote).run(
      accountId: state.account.id, network: .wifi(constrained: false))
    let rejected = try await store.read(accountId: state.account.id)
    #expect(rejected.operations.first { $0.id == publish.id }?.receiptRejected == true)
    #expect(!rejected.publicationMayProceed(intent))
    if cancel {
      try await repository.cancelPublication(accountId: state.account.id, intentId: intent.id)
    } else {
      _ = try await repository.queuePublication(accountId: state.account.id, voyageId: voyage.id)
    }
    let saved = try await store.read(accountId: state.account.id)
    #expect(saved.publicationIntents.first { $0.id == intent.id }?.cancellationPending == false)
    let previous = try #require(saved.operations.first { $0.id == publish.id })
    #expect(previous.state == .canceled)
    #expect(previous.request == publish.request)
    #expect(
      saved.operations.filter { if case .cancelPublication = $0.kind { true } else { false } }
        .allSatisfy { $0.state == .canceled })
  }
}
