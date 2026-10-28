import CryptoKit
import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func draftRecoveryUsesVerifiedFilesAndDistinguishesProcessedCopiesFromOriginals() async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory)
  var state = workspaceWithPhotos(remote: true)
  let bytes = Data([0xff, 0xd8, 0xff, 0xe0, 1, 2, 3])
  let hash = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
  state.photos[0].source = SourceFile(
    relativePath: "originals/retained", contentHash: hash, byteCount: Int64(bytes.count),
    mediaType: "image/jpeg")
  try await vault.write(bytes, relativePath: "originals/retained", accountId: state.account.id)
  state.photos[1].source = SourceFile(
    relativePath: "originals/corrupt", contentHash: String(repeating: "0", count: 64), byteCount: 7,
    mediaType: "image/jpeg")
  try await vault.write(bytes, relativePath: "originals/corrupt", accountId: state.account.id)
  let processedPath = "downloads/processed-display-\(hash).webp"
  state.photos[1].offlineCopy = OfflinePhotoCopy(
    revision: "7", variants: ["display": processedPath], byteCount: Int64(bytes.count))
  state.photos[1].coordinate = Coordinate(latitude: 35, longitude: 139)
  state.photos[1].capturedAt = "2026:10:01 12:00:00"
  try await vault.write(bytes, relativePath: processedPath, accountId: state.account.id)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let service = DraftRecoveryService(store: store, vault: vault)
  let id = try await service.recover(accountId: state.account.id, voyageId: state.voyages[0].id)
  let saved = try await store.read(accountId: state.account.id)
  let photos = saved.photos(for: id)
  #expect(photos.count == 2)
  #expect(photos[0].filename == state.photos[0].filename)
  #expect(photos[0].coordinate == state.photos[0].coordinate)
  #expect(photos[1].filename == "Recovered processed copy")
  #expect(photos[1].coordinate == nil)
  #expect(photos[1].capturedAt == nil)
  #expect(
    photos.allSatisfy {
      $0.source?.relativePath.hasPrefix("originals/") == true && $0.serverId == nil
    })
  #expect(Set(photos.map(\.uploadKey)).isDisjoint(with: state.photos.map(\.uploadKey)))
  #expect(saved.voyages.first { $0.id == id }?.confirmedPublished == false)
  #expect(saved.operations.filter { $0.voyageId == id }.count == 3)
  #expect(saved.publicationIntents.isEmpty)
  try await vault.remove(processedPath, accountId: state.account.id)
  #expect(
    try await vault.read(photos[1].source!.relativePath, accountId: state.account.id) == bytes)
}

@Test func draftRecoveryRejectsLockedAccountsAndKeepsMissingFilesOutOfTheNewQueue() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory)
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = workspaceWithPhotos(remote: true)
  try await store.insert(state)
  let service = DraftRecoveryService(store: store, vault: vault)
  let id = try await service.recover(accountId: state.account.id, voyageId: state.voyages[0].id)
  #expect(try await store.read(accountId: state.account.id).photos(for: id).isEmpty)
  try await LocalRepository(store: store).lock(accountId: state.account.id)
  await #expect(throws: WorkspaceError.locked) {
    try await service.recover(accountId: state.account.id, voyageId: state.voyages[0].id)
  }
}
