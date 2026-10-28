import CryptoKit
import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

private func storageFixture() async throws -> (CoreDataWorkspaceStore, FileVault, Workspace) {
  let root = try temporaryDirectory()
  let vault = try FileVault(root: root)
  let store = try CoreDataWorkspaceStore(url: nil)
  var state = workspaceWithPhotos(remote: true)
  for index in state.photos.indices {
    let photo = state.photos[index]
    let bytes = Data("verified display \(index)".utf8)
    let hash = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    let path = "downloads/\(photo.id)/display-\(hash).webp"
    try await vault.write(bytes, relativePath: path, accountId: state.account.id)
    try await vault.write(
      Data(repeating: 65, count: 10), relativePath: photo.source!.relativePath,
      accountId: state.account.id)
    state.photos[index].offlineCopy = OfflinePhotoCopy(
      revision: "7", variants: ["display": path], byteCount: Int64(bytes.count))
  }
  try await store.insert(state)
  return (store, vault, state)
}

@Test func originalCleanupRequiresReadyAcceptedPhotosAndVerifiedOwnerFiles() async throws {
  let (store, vault, state) = try await storageFixture()
  let root = await vault.root
  defer { try? FileManager.default.removeItem(at: root) }
  let account = state.account.id
  let failed = state.photos[1].id
  let damaged = state.photos[2].id
  try await store.update(accountId: account) { current in
    current.photos[current.photos.firstIndex { $0.id == failed }!].variant = .failed
  }
  try await vault.write(
    Data("damaged".utf8), relativePath: state.photos[2].offlineCopy!.variants["display"]!,
    accountId: account)
  let storage = StorageManager(store: store, vault: vault)
  let before = try await storage.breakdown(accountId: account)
  #expect(before.keptOriginals == 20)
  #expect(before.unsyncedPhotos == 10)
  #expect(before.offlineVoyages > 0)
  #expect(try await storage.removeUploadedOriginals(accountId: account) == 10)
  let saved = try await store.read(accountId: account)
  #expect(saved.photos.first { $0.id == state.photos[0].id }?.source == nil)
  #expect(saved.photos.first { $0.id == failed }?.source != nil)
  #expect(saved.photos.first { $0.id == damaged }?.source != nil)
  await #expect(throws: (any Error).self) {
    try await vault.read(state.photos[0].source!.relativePath, accountId: account)
  }
  #expect(
    try await vault.read(state.photos[1].source!.relativePath, accountId: account).count == 10)
  #expect(try await storage.removeUploadedOriginals(accountId: account) == 0)
}

@Test func cleanupNeverDeletesSharedSourcesNeededByPendingOrUncertainWork() async throws {
  let (store, vault, state) = try await storageFixture()
  let root = await vault.root
  defer { try? FileManager.default.removeItem(at: root) }
  let account = state.account.id
  let path = state.photos[0].source!.relativePath
  try await store.update(accountId: account) { current in
    var pending = LocalPhoto(voyageId: UUID(), filename: "Shared pending source")
    pending.source = state.photos[0].source
    current.photos.append(pending)
    var uncertain = OutboxOperation(
      accountId: account, generation: 0, voyageId: state.voyages[0].id,
      kind: .upload(photoId: state.photos[1].id, publicConsent: false))
    uncertain.state = .uncertain
    current.operations.append(uncertain)
  }
  let storage = StorageManager(store: store, vault: vault)
  #expect(try await storage.removeUploadedOriginals(accountId: account) == 0)
  #expect(try await vault.read(path, accountId: account).count == 10)
  try await LocalRepository(store: store).lock(accountId: account)
  await #expect(throws: WorkspaceError.locked) {
    try await storage.removeUploadedOriginals(accountId: account)
  }
}

@Test func voyageExportListsMissingOriginalsAndIncludesOnlyTheSelectedVoyage() async throws {
  let (store, vault, initial) = try await storageFixture()
  let root = await vault.root
  defer { try? FileManager.default.removeItem(at: root) }
  var state = initial
  let other = LocalVoyage(
    ownerId: state.account.id, fields: VoyageFields(title: "Other", description: "Private"))
  state.voyages.append(other)
  state.photos.append(LocalPhoto(voyageId: other.id, filename: "Other.jpg"))
  try await vault.remove(state.photos[0].source!.relativePath, accountId: state.account.id)
  let preview = try await vault.exportPreview(state, voyageId: state.voyages[0].id)
  #expect(preview.missingOriginals == [state.photos[0].filename])
  #expect(preview.includedFiles == 2)
  let destination = root.appendingPathComponent("export")
  _ = try await vault.export(state, voyageId: state.voyages[0].id, destination: destination)
  let exported = try JSONDecoder().decode(
    Workspace.self, from: Data(contentsOf: destination.appendingPathComponent("workspace.json")))
  #expect(exported.voyages.count == 1)
  #expect(exported.photos.count == 3)
  #expect(exported.photos.allSatisfy { $0.voyageId == state.voyages[0].id })
  #expect(
    FileManager.default.fileExists(
      atPath: destination.appendingPathComponent("export-preview.json").path))
  _ = store
}

actor SuspendedUpdateStore: WorkspaceStore {
  let base: any WorkspaceStore
  var first = true
  var paused = false
  var resume: CheckedContinuation<Void, Never>?
  var observer: CheckedContinuation<Void, Never>?
  init(_ base: any WorkspaceStore) { self.base = base }
  func accountIds() async throws -> [UUID] { try await base.accountIds() }
  func insert(_ workspace: Workspace) async throws { try await base.insert(workspace) }
  func read(accountId: UUID) async throws -> Workspace { try await base.read(accountId: accountId) }
  func remove(accountId: UUID) async throws { try await base.remove(accountId: accountId) }
  func waitUntilPaused() async {
    if paused { return }
    await withCheckedContinuation { observer = $0 }
  }
  func continueUpdate() {
    resume?.resume()
    resume = nil
  }
  func update(accountId: UUID, _ change: @escaping @Sendable (inout Workspace) throws -> Void)
    async throws
  {
    if first {
      first = false
      paused = true
      observer?.resume()
      observer = nil
      await withCheckedContinuation { resume = $0 }
    }
    try await base.update(accountId: accountId, change)
  }
}

@Test func sourceReuseCannotCommitAnOldReferenceAfterExplicitOriginalCleanup() async throws {
  let (store, vault, state) = try await storageFixture()
  let root = await vault.root
  defer { try? FileManager.default.removeItem(at: root) }
  let delayed = SuspendedUpdateStore(store)
  let reuse = Task {
    try await LocalRepository(store: delayed).addAgain(
      accountId: state.account.id, photoId: state.photos[0].id)
  }
  await delayed.waitUntilPaused()
  _ = try await StorageManager(store: delayed, vault: vault).removeUploadedOriginals(
    accountId: state.account.id)
  await delayed.continueUpdate()
  await #expect(throws: FileVaultError.missingSource) { try await reuse.value }
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos.count == state.photos.count)
}

@Test func aVoyageExportUsesASystemZipSnapshotThatRemainsReadableAfterCoordinationEnds()
  async throws
{
  let (_, vault, state) = try await storageFixture()
  let root = await vault.root
  defer { try? FileManager.default.removeItem(at: root) }
  let archive = try await vault.exportArchive(
    state, voyageId: state.voyages[0].id,
    destination: root.appendingPathComponent("archive"))
  #expect(archive.pathExtension == "zip")
  let file = try FileHandle(forReadingFrom: archive)
  defer { try? file.close() }
  #expect(try file.read(upToCount: 4) == Data([0x50, 0x4b, 0x03, 0x04]))
  #expect(try archive.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0 > 0)
}

@Test func exportingAnArchiveNeverOverwritesAnExistingFileOrRemovesOriginalsAfterAFailure()
  async throws
{
  let (_, vault, state) = try await storageFixture()
  let root = await vault.root
  defer { try? FileManager.default.removeItem(at: root) }
  let destination = root.appendingPathComponent("existing")
  let archive = destination.appendingPathExtension("zip")
  let previous = Data("Keep this existing archive".utf8)
  try previous.write(to: archive)
  await #expect(throws: (any Error).self) {
    try await vault.exportArchive(state, voyageId: state.voyages[0].id, destination: destination)
  }
  #expect(try Data(contentsOf: archive) == previous)
  #expect(
    try await vault.read(state.photos[0].source!.relativePath, accountId: state.account.id).count
      == 10)
}
