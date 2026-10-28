import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

private actor CancellationProbe {
  var accounts: [UUID] = []
  func cancel(_ id: UUID) { accounts.append(id) }
}

@Test func localOnlyDeletionCancelsBeforeRemovingFilesAndProtectsSharedReferences() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory)
  var state = workspaceWithPhotos()
  let deletedId = state.voyages[0].id
  let other = LocalVoyage(ownerId: state.account.id, fields: state.voyages[0].fields)
  state.voyages.append(other)
  var shared = LocalPhoto(voyageId: other.id, filename: "Shared original")
  shared.source = state.photos[0].source
  state.photos.append(shared)
  for photo in state.photos {
    if let source = photo.source {
      try await vault.write(
        Data([1, 2, 3]), relativePath: source.relativePath, accountId: state.account.id)
    }
  }
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let original = state
  let probe = CancellationProbe()
  let service = LocalDiscardService(store: store, vault: vault) { id in
    let tombstone = try? await store.read(accountId: id)
    #expect(tombstone?.voyages.first { $0.id == deletedId }?.deleted == true)
    #expect(
      (try? await vault.read(original.photos[1].source!.relativePath, accountId: id))
        == Data([1, 2, 3]))
    await probe.cancel(id)
  }
  try await service.deleteLocalOnly(accountId: state.account.id, voyageId: deletedId)
  #expect(await probe.accounts == [state.account.id])
  let saved = try await store.read(accountId: state.account.id)
  #expect(
    saved.photos.filter { $0.voyageId == deletedId }.allSatisfy { $0.source == nil && $0.deleted })
  #expect(
    try await vault.read(shared.source!.relativePath, accountId: state.account.id)
      == Data([1, 2, 3]))
  #expect(
    !FileManager.default.fileExists(
      atPath: try await vault.url(state.photos[1].source!.relativePath, accountId: state.account.id)
        .path))
}

@Test
func localDeletionRejectsRemoteAndAmbiguousCreationAndUnavailableDiscardKeepsImmutableEvidence()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory)
  var state = workspaceWithPhotos()
  var operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: state.voyages[0].id,
    kind: .create(state.voyages[0].fields))
  operation.request = MutationRequest(method: "POST", path: "/possibly-created")
  operation.state = .uncertain
  state.operations = [operation]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let service = LocalDiscardService(store: store, vault: vault) { _ in }
  await #expect(throws: ValidationError.self) {
    try await service.deleteLocalOnly(accountId: state.account.id, voyageId: state.voyages[0].id)
  }
  try await store.update(accountId: state.account.id) { current in
    current.operations[0].state = .conflict
  }
  try await service.discardUnavailable(accountId: state.account.id, voyageId: state.voyages[0].id)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations[0].request == operation.request)
  #expect(saved.operations[0].state == .canceled)
  #expect(saved.voyages[0].deleted)
  try await LocalRepository(store: store).lock(accountId: state.account.id)
  await #expect(throws: WorkspaceError.locked) {
    try await service.discardUnavailable(accountId: state.account.id, voyageId: state.voyages[0].id)
  }
}

@Test func interruptedLocalFileDeletionResumesWithoutRemovingReferencedFiles() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory)
  var state = workspaceWithPhotos()
  let shared = state.photos[0].source!.relativePath
  state.pendingFileRemovals = ["originals/unreferenced", shared]
  try await vault.write(
    Data([1]), relativePath: "originals/unreferenced", accountId: state.account.id)
  try await vault.write(Data([2]), relativePath: shared, accountId: state.account.id)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let service = LocalDiscardService(store: store, vault: vault) { _ in }
  try await service.finishPendingRemovals(accountId: state.account.id)
  #expect(try await store.read(accountId: state.account.id).pendingFileRemovals?.isEmpty == true)
  #expect(
    !FileManager.default.fileExists(
      atPath: try await vault.url("originals/unreferenced", accountId: state.account.id).path))
  #expect(try await vault.read(shared, accountId: state.account.id) == Data([2]))
  try await store.update(accountId: state.account.id) { $0.pendingFileRemovals = ["../outside"] }
  await #expect(throws: FileVaultError.invalidPath) {
    try await service.finishPendingRemovals(accountId: state.account.id)
  }
  #expect(try await store.read(accountId: state.account.id).pendingFileRemovals == ["../outside"])
}
