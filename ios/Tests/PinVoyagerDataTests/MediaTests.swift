import Foundation
import ImageIO
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func preparesPreviewAndPreservesOriginalMetadata() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let fixture = Fixtures.directory("images")
  let files = try FileManager.default.contentsOfDirectory(
    at: fixture, includingPropertiesForKeys: nil)
  let jpeg = try #require(
    files.first { ["jpg", "jpeg", "heic"].contains($0.pathExtension.lowercased()) })
  let vault = try FileVault(root: directory)
  let accountId = UUID()
  let photo = try await PhotoPreparer(vault: vault).prepare(
    jpeg, accountId: accountId, voyageId: UUID(), maxBytes: 25 * 1_024 * 1_024)
  #expect(photo.source != nil)
  #expect(photo.width > 0 && photo.height > 0)
  let preview = try await vault.read(try #require(photo.previewPath), accountId: accountId)
  let image = try #require(CGImageSourceCreateWithData(preview as CFData, nil))
  let props = try #require(CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any])
  #expect(
    max(props[kCGImagePropertyPixelWidth] as! Int, props[kCGImagePropertyPixelHeight] as! Int)
      <= 512)
  #expect(
    try await vault.read(photo.source!.relativePath, accountId: accountId) == Data(contentsOf: jpeg)
  )
}

@Test func invalidImageNeverProducesSavedRow() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("bad.jpg")
  try Data([0xff, 0xd8, 0xff, 0]).write(to: url)
  let preparer = PhotoPreparer(
    vault: try FileVault(root: directory.appendingPathComponent("vault")))
  await #expect(throws: (any Error).self) {
    try await preparer.prepare(url, accountId: UUID(), voyageId: UUID(), maxBytes: 100)
  }
}

@Test func completedOriginalsRecoverAfterTheFileDatabaseBoundaryWithoutChangingUploadIdentity()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = workspaceWithPhotos()
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let photoId = try await repository.beginImport(
    accountId: state.account.id, voyageId: state.voyages[0].id, filename: "Recovered photo.jpg")
  let placeholder = try #require(
    try await store.read(accountId: state.account.id).photos.first { $0.id == photoId })
  let vault = try FileVault(root: directory)
  let fixture = try #require(
    try FileManager.default.contentsOfDirectory(
      at: Fixtures.directory("images"), includingPropertiesForKeys: nil
    ).first { ["jpg", "jpeg", "heic"].contains($0.pathExtension) })
  _ = try await vault.importFile(
    fixture, accountId: state.account.id, photoId: photoId, maxBytes: 25_000_000)
  try await ImportRecovery(store: store, vault: vault).recover(
    accountId: state.account.id, maxBytes: 25_000_000)
  let recovered = try await store.read(accountId: state.account.id)
  let photo = try #require(recovered.photos.first { $0.id == photoId })
  #expect(photo.transfer == .saved)
  #expect(photo.uploadKey == placeholder.uploadKey)
  #expect(photo.filename == placeholder.filename)
  #expect(photo.previewPath != nil)
  #expect(photo.source != nil)
  #expect(recovered.operations.count == 1)
  try await ImportRecovery(store: store, vault: vault).recover(
    accountId: state.account.id, maxBytes: 25_000_000)
  #expect(try await store.read(accountId: state.account.id).operations.count == 1)
}

@Test func importRecoveryRetainsMissingOrDamagedSourcesAndHonorsAccountLocks() async throws {
  for locked in [false, true] {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = try CoreDataWorkspaceStore(url: nil)
    let state = workspaceWithPhotos()
    try await store.insert(state)
    let repository = LocalRepository(store: store)
    let missing = try await repository.beginImport(
      accountId: state.account.id, voyageId: state.voyages[0].id, filename: "Missing.jpg")
    let damaged = try await repository.beginImport(
      accountId: state.account.id, voyageId: state.voyages[0].id, filename: "Damaged.jpg")
    let vault = try FileVault(root: directory)
    try await vault.write(
      Data([0xff, 0xd8, 0xff, 0]), relativePath: "originals/\(damaged)", accountId: state.account.id
    )
    if locked { try await repository.lock(accountId: state.account.id) }
    try await ImportRecovery(store: store, vault: vault).recover(
      accountId: state.account.id, maxBytes: 25_000_000)
    let saved = try await store.read(accountId: state.account.id)
    for id in [missing, damaged] {
      #expect(saved.photos.first { $0.id == id }?.transfer == (locked ? .acquiring : .needsSource))
    }
    #expect(saved.operations.isEmpty)
    #expect(try await vault.read("originals/\(damaged)", accountId: state.account.id).count == 4)
  }
}

@Test func explicitlySmallerCopiesKeepTheImportedOriginalAndPrivateLocation() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let context = try #require(
    CGContext(
      data: nil, width: 2048, height: 2048, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
  context.setFillColor(CGColor(red: 0.1, green: 0.6, blue: 0.6, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: 2048, height: 2048))
  let image = try #require(context.makeImage())
  let input = directory.appendingPathComponent("Large.jpg")
  let destination = try #require(
    CGImageDestinationCreateWithURL(input as CFURL, "public.jpeg" as CFString, 1, nil))
  CGImageDestinationAddImage(
    destination, image,
    [
      kCGImagePropertyGPSDictionary: [
        kCGImagePropertyGPSLatitude: 33.5, kCGImagePropertyGPSLatitudeRef: "S",
        kCGImagePropertyGPSLongitude: 151.25, kCGImagePropertyGPSLongitudeRef: "E",
      ]
    ] as CFDictionary)
  #expect(CGImageDestinationFinalize(destination))
  let vault = try FileVault(root: directory.appendingPathComponent("vault"))
  var state = workspaceWithPhotos()
  state.photos = []
  let original = try await PhotoPreparer(vault: vault).prepare(
    input, accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 25_000_000)
  state.photos = [original]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let replacement = try await PhotoImportService(store: store, vault: vault).makeSmallerCopy(
    accountId: state.account.id, photoId: original.id, maxBytes: 25_000_000, maxDimension: 64)
  let saved = try await store.read(accountId: state.account.id)
  let copy = try #require(saved.photos.first { $0.id == replacement })
  #expect(copy.width <= 64 && copy.height <= 64)
  #expect(copy.source!.byteCount < original.source!.byteCount)
  #expect(copy.source!.contentHash != original.source!.contentHash)
  #expect(copy.uploadKey != original.uploadKey)
  #expect(copy.coordinate == original.coordinate)
  #expect(
    try await vault.read(original.source!.relativePath, accountId: state.account.id)
      == Data(contentsOf: input))
  let exported = try await vault.export(
    saved, destination: directory.appendingPathComponent("export"))
  #expect(
    FileManager.default.fileExists(
      atPath: exported.appendingPathComponent(original.id.uuidString).path))

}
