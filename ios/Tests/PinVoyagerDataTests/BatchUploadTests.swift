import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

private actor BatchUploads: UploadSubmitting {
  var submitted: Set<UUID> = []
  func submit(operation: OutboxOperation, request: MutationRequest, workspace: Workspace) {
    if case .upload(let id, _) = operation.kind { submitted.insert(id) }
  }
  func cancel(accountId: UUID) {}
}

@Test func cellularConsentAppliesOnlyToOneImportAndExpiresAfterReceiptAcceptance() async throws {
  var state = workspaceWithPhotos()
  state.voyages[0].serverId = UUID()
  state.voyages[0].revision = "1"
  let account = state.account.id
  let voyage = state.voyages[0].id
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let allowed = try await repository.beginImports(
    accountId: account, voyageId: voyage,
    filenames: ["Allowed one.jpg", "Allowed two.jpg"], cellularException: true)
  let ordinary = try await repository.beginImports(
    accountId: account, voyageId: voyage, filenames: ["Wi-Fi only.jpg"])
  let pending = try await store.read(accountId: account)
  let batch = try #require(pending.photos.first { $0.id == allowed[0] }?.importBatchId)
  #expect(pending.photos.first { $0.id == allowed[1] }?.importBatchId == batch)
  #expect(pending.photos.first { $0.id == ordinary[0] }?.importBatchId != batch)
  for id in allowed + ordinary {
    var photo = LocalPhoto(id: id, voyageId: voyage, filename: "Prepared photo")
    photo.source = state.photos[0].source
    try await repository.addPhoto(accountId: account, photo: photo)
  }
  let uploads = BatchUploads()
  let remote = StubSyncRemote()
  let sync = SyncCoordinator(store: store, remote: remote, uploads: uploads)
  try await sync.run(accountId: account, network: .cellular(constrained: true))
  #expect(await uploads.submitted.isEmpty)
  try await sync.run(accountId: account, network: .cellular(constrained: false))
  #expect(await uploads.submitted == Set(allowed))
  let submitted = try await store.read(accountId: account)
  for (index, id) in allowed.enumerated() {
    let photo = try #require(submitted.photos.first { $0.id == id })
    #expect(photo.cellularException == true)
    await remote.setReceipt(
      .applied(targetId: UUID(), revision: String(index + 2), previousRevision: String(index + 1)),
      id: photo.uploadKey)
  }
  try await sync.run(accountId: account, network: .cellular(constrained: false))
  let saved = try await store.read(accountId: account)
  #expect(saved.policy.wifiOnly)
  #expect(
    saved.photos.filter { allowed.contains($0.id) }.allSatisfy {
      $0.cellularException != true && $0.serverId != nil
    })
  #expect(saved.photos.first { $0.id == ordinary[0] }?.serverId == nil)
  let generation = saved.account.generation
  try await repository.setCellularException(accountId: account, batchId: batch, allowed: true)
  #expect(try await store.read(accountId: account).account.generation == generation)
}

@Test func revokingBatchCellularPermissionChangesTheGenerationWithoutChangingGlobalPolicy()
  async throws
{
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let ids = try await repository.beginImports(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    filenames: ["One.jpg", "Two.jpg"], cellularException: true)
  let pending = try await store.read(accountId: state.account.id)
  let batch = try #require(pending.photos.first { $0.id == ids[0] }?.importBatchId)
  try await repository.setCellularException(
    accountId: state.account.id, batchId: batch, allowed: false)
  let revoked = try await store.read(accountId: state.account.id)
  #expect(revoked.account.generation == pending.account.generation + 1)
  #expect(revoked.policy.wifiOnly)
  #expect(revoked.photos.filter { ids.contains($0.id) }.allSatisfy { $0.cellularException != true })
  let oldKeys = Set(pending.photos.filter { ids.contains($0.id) }.map(\.uploadKey))
  let newKeys = Set(revoked.photos.filter { ids.contains($0.id) }.map(\.uploadKey))
  #expect(newKeys == oldKeys)

}

@Test func globalUploadPauseStopsBytesWithoutPausingMetadataOrReceiptRecovery() async throws {
  var state = workspaceWithPhotos(remote: true)
  var upload = OutboxOperation(
    accountId: state.account.id, generation: 0,
    voyageId: state.voyages[0].id, kind: .upload(photoId: state.photos[0].id, publicConsent: false))
  upload.state = .uncertain
  upload.request = MutationRequest(method: "POST", path: "/submitted-photo")
  state.operations = [upload]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  try await repository.setPreference(
    accountId: state.account.id, preference: .uploadsPaused, value: true)
  let paused = try await store.read(accountId: state.account.id)
  #expect(!paused.policy.paused)
  #expect(paused.uploadPolicy(for: state.photos[0].id).paused)
  try await repository.editVoyage(
    accountId: state.account.id, voyageId: state.voyages[0].id,
    fields: VoyageFields(
      title: "Metadata still synchronizes", description: "Only photo bytes are paused"))
  let remote = StubSyncRemote()
  await remote.setReceipt(
    .applied(targetId: state.photos[0].serverId!, revision: "8", previousRevision: "7"),
    id: state.photos[0].uploadKey)
  await remote.setOutcomes([
    .applied(targetId: state.voyages[0].serverId!, revision: "9", previousRevision: "8")
  ])
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations.allSatisfy { $0.state == .applied })
  #expect(saved.policy.areUploadsPaused)
  #expect(await remote.submissions.count == 1)
}

@Test func receiptRecoveryIncludesEverySubmittedPhotoAfterALargeCompletedHistory() async throws {
  var state = workspaceWithPhotos(remote: true)
  let voyage = state.voyages[0]
  state.photos = []
  state.policy.uploadsPaused = true
  func id(_ number: Int) -> UUID {
    UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", number))!
  }
  for number in 1...200 {
    var completed = OutboxOperation(
      id: id(number), accountId: state.account.id, generation: 0,
      voyageId: voyage.id, kind: .edit(voyage.fields))
    completed.state = .applied
    completed.request = MutationRequest(method: "PATCH", path: "/completed")
    state.operations.append(completed)
  }
  let remote = StubSyncRemote()
  for number in 0..<150 {
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Submitted photo \(number)")
    photo.transfer = .checkingReceipt
    state.photos.append(photo)
    var upload = OutboxOperation(
      id: id(number + 900), accountId: state.account.id, generation: 0,
      voyageId: voyage.id, kind: .upload(photoId: photo.id, publicConsent: false))
    upload.state = .uncertain
    upload.request = MutationRequest(method: "POST", path: "/submitted-photo")
    state.operations.append(upload)
    await remote.setReceipt(
      .applied(
        targetId: UUID(), revision: String(number + 8), previousRevision: String(number + 7)),
      id: photo.uploadKey)
  }
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  try await SyncCoordinator(store: store, remote: remote).run(
    accountId: state.account.id, network: .cellular(constrained: false))
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos.allSatisfy { $0.serverId != nil && $0.transfer == .accepted })
  #expect(saved.operations.allSatisfy { $0.state == .applied })
  #expect(await remote.submissions.isEmpty)
}
