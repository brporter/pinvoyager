import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

private struct FixturePhotoSource: PhotoAcquiring {
  let filename: String
  let fails: Bool
  func acquire() async throws -> URL {
    if fails { throw FileVaultError.missingSource }
    let file = Fixtures.directory("images").appendingPathComponent("quadrants-rgb.heic")
    let owned = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      .appendingPathExtension("heic")
    try FileManager.default.copyItem(at: file, to: owned)
    return owned
  }
}

@Test func photoBatchKeepsEveryPlaceholderAndContinuesAfterProviderFailure() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = workspaceWithPhotos()
  try await store.insert(state)
  let importer = PhotoImportService(store: store, vault: try FileVault(root: directory))
  let result = try await importer.importBatch(
    [
      FixturePhotoSource(filename: "First.heic", fails: false),
      FixturePhotoSource(filename: "From iCloud.heic", fails: true),
      FixturePhotoSource(filename: "Last.heic", fails: false),
    ], accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 25_000_000)
  #expect(result.saved.count == 2)
  #expect(result.failed.count == 1)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos.count == state.photos.count + 3)
  #expect(saved.operations.count == 2)
  #expect(saved.photos.first { $0.id == result.failed[0] }?.transfer == .needsSource)
  for id in result.saved {
    let photo = try #require(saved.photos.first { $0.id == id })
    #expect(photo.source != nil)
    #expect(photo.previewPath != nil)
    #expect(photo.transfer == .saved)
  }
}

@Test func stoppingAnImportKeepsTheCurrentSavedPhotoAndMarksRemainingSources() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = workspaceWithPhotos()
  try await store.insert(state)
  let importer = PhotoImportService(store: store, vault: try FileVault(root: directory))
  let result = try await importer.importBatch(
    (0..<3).map { FixturePhotoSource(filename: "Photo \($0)", fails: false) },
    accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 25_000_000
  ) { event in
    if case .saved = event { await importer.cancel() }
  }
  #expect(result.saved.count == 1)
  #expect(result.failed.count == 2)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations.count == 1)
  #expect(
    saved.photos.filter { result.failed.contains($0.id) }.allSatisfy { $0.transfer == .needsSource }
  )
}

@Test func importBatchRequiresPublicConsentAndEnforcesItsSelectionLimitBeforeAcquisition()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let store = try CoreDataWorkspaceStore(url: nil)
  var state = workspaceWithPhotos(remote: true)
  state.voyages[0].confirmedPublished = true
  try await store.insert(state)
  let importer = PhotoImportService(store: store, vault: try FileVault(root: directory))
  let source = FixturePhotoSource(filename: "Public.heic", fails: false)
  await #expect(throws: ValidationError.self) {
    try await importer.importBatch(
      [source], accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 25_000_000)
  }
  await #expect(throws: ValidationError.self) {
    try await importer.importBatch(
      Array(repeating: source, count: 101), accountId: state.account.id,
      voyageId: state.voyages[0].id, maxBytes: 25_000_000, publicConsent: true)
  }
  let result = try await importer.importBatch(
    [source], accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 25_000_000,
    publicConsent: true)
  #expect(result.saved.count == 1)
  let saved = try await store.read(accountId: state.account.id)
  if case .upload(_, let consent) = try #require(saved.operations.first).kind {
    #expect(consent)
  } else {
    Issue.record("Expected a consented upload")
  }
}

@Test func explicitSourceReplacementUsesFreshKeysAndKeepsTheOriginalUntilTheCopyIsDurable()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = workspaceWithPhotos(remote: true)
  try await store.insert(state)
  let importer = PhotoImportService(store: store, vault: try FileVault(root: directory))
  await #expect(throws: FileVaultError.self) {
    try await importer.replace(
      FixturePhotoSource(filename: "Unavailable", fails: true), accountId: state.account.id,
      photoId: state.photos[0].id, maxBytes: 25_000_000)
  }
  #expect(
    try await store.read(accountId: state.account.id).photos.first { $0.id == state.photos[0].id }?
      .deleted == false)
  let replacement = try await importer.replace(
    FixturePhotoSource(filename: "Replacement.heic", fails: false), accountId: state.account.id,
    photoId: state.photos[0].id, maxBytes: 25_000_000)
  let saved = try await store.read(accountId: state.account.id)
  let newPhoto = try #require(saved.photos.first { $0.id == replacement })
  #expect(newPhoto.uploadKey != state.photos[0].uploadKey)
  #expect(newPhoto.source != nil)
  #expect(newPhoto.order == state.photos[0].order)
  #expect(saved.photos.first { $0.id == state.photos[0].id }?.deleted == true)
  #expect(saved.photos.first { $0.id == state.photos[0].id }?.source == state.photos[0].source)
  #expect(
    saved.operations.contains {
      if case .removePhoto(let id, _) = $0.kind { return id == state.photos[0].id }
      return false
    })
  #expect(
    saved.operations.contains {
      if case .order = $0.kind { return true }
      return false
    })
}
