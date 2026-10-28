import Foundation
import PinVoyagerDomain

/// Acquisition returns a temporary local file owned by the importer, which removes it after copying.
public protocol PhotoAcquiring: Sendable {
  var filename: String { get }
  func acquire() async throws -> URL
}
public enum PhotoImportEvent: Sendable {
  case preparing(index: Int, total: Int)
  case saved(UUID)
  case failed(UUID, String)
}
public struct PhotoImportResult: Sendable {
  public var saved: [UUID] = []
  public var failed: [UUID] = []
}

/// Each selected photo gets a durable identity before its provider is asked for bytes.
public actor PhotoImportService {
  private let store: any WorkspaceStore
  private let repository: LocalRepository
  private let preparer: PhotoPreparer
  private let vault: FileVault
  private var running = false
  private var stopped = false
  public init(store: any WorkspaceStore, vault: FileVault) {
    self.store = store
    self.vault = vault
    repository = LocalRepository(store: store)
    preparer = PhotoPreparer(vault: vault)
  }
  public func cancel() { stopped = true }

  public func importBatch(
    _ sources: [any PhotoAcquiring], accountId: UUID, voyageId: UUID,
    maxBytes: Int64, publicConsent: Bool = false, cellularException: Bool = false,
    progress: @Sendable (PhotoImportEvent) async -> Void = { _ in }
  ) async throws -> PhotoImportResult {
    guard !running, sources.count <= 100 else {
      throw ValidationError(
        field: "photos",
        message: "Finish the current import and choose at most 100 photographs at once.")
    }
    running = true
    stopped = false
    defer { running = false }
    let initial = try await store.read(accountId: accountId)
    let pending = try await repository.beginImports(
      accountId: accountId, voyageId: voyageId,
      filenames: sources.map(\.filename), publicConsent: publicConsent,
      expectedGeneration: initial.account.generation, cellularException: cellularException)
    var result = PhotoImportResult()
    for (index, source) in sources.enumerated() {
      let id = pending[index]
      do {
        guard !stopped, !Task.isCancelled else { throw CancellationError() }
        await progress(.preparing(index: index + 1, total: sources.count))
        let file = try await source.acquire()
        defer { try? FileManager.default.removeItem(at: file) }
        _ = try await saveFile(
          file, accountId: accountId, voyageId: voyageId, maxBytes: maxBytes,
          pendingId: id, generation: initial.account.generation)
        result.saved.append(id)
        await progress(.saved(id))
      } catch {
        let message =
          error is CancellationError
          ? "Import stopped. Choose this photo again." : error.localizedDescription
        try await repository.markImportMissing(accountId: accountId, photoId: id, message: message)
        result.failed.append(id)
        await progress(.failed(id, message))
        if error is WorkspaceError { stopped = true }
      }
    }
    return result
  }

  @discardableResult public func saveFile(
    _ file: URL, accountId: UUID, voyageId: UUID,
    maxBytes: Int64, publicConsent: Bool = false, cellularException: Bool = false
  ) async throws -> UUID {
    let state = try await store.read(accountId: accountId)
    let id = try await repository.beginImports(
      accountId: accountId, voyageId: voyageId,
      filenames: [file.lastPathComponent], publicConsent: publicConsent,
      expectedGeneration: state.account.generation, cellularException: cellularException)[0]
    do {
      return try await saveFile(
        file, accountId: accountId, voyageId: voyageId, maxBytes: maxBytes,
        pendingId: id, generation: state.account.generation)
    } catch {
      try await repository.markImportMissing(
        accountId: accountId, photoId: id, message: error.localizedDescription)
      throw error
    }
  }

  @discardableResult public func replace(
    _ source: any PhotoAcquiring, accountId: UUID, photoId: UUID,
    maxBytes: Int64, publicConsent: Bool = false
  ) async throws -> UUID {
    let state = try await store.read(accountId: accountId)
    guard let original = state.photos.first(where: { $0.id == photoId && !$0.deleted }) else {
      throw FileVaultError.missingSource
    }
    let id = try await repository.beginReplacement(
      accountId: accountId, photoId: photoId, filename: source.filename,
      publicConsent: publicConsent, expectedGeneration: state.account.generation)
    do {
      let file = try await source.acquire()
      defer { try? FileManager.default.removeItem(at: file) }
      return try await saveFile(
        file, accountId: accountId, voyageId: original.voyageId, maxBytes: maxBytes,
        pendingId: id, generation: state.account.generation)
    } catch {
      try await repository.markImportMissing(
        accountId: accountId, photoId: id, message: error.localizedDescription)
      throw error
    }
  }

  @discardableResult public func makeSmallerCopy(
    accountId: UUID, photoId: UUID, maxBytes: Int64,
    maxDimension: Int = 2048, publicConsent: Bool = false
  ) async throws -> UUID {
    let state = try await store.read(accountId: accountId)
    guard state.account.access != .locked,
      let photo = state.photos.first(where: { $0.id == photoId && !$0.deleted }),
      let source = photo.source
    else { throw FileVaultError.missingSource }
    let input = try await vault.url(source.relativePath, accountId: accountId)
    let file = try await preparer.smallerCopy(input, maxDimension: maxDimension)
    defer { try? FileManager.default.removeItem(at: file) }
    let count = try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
    guard count < source.byteCount else {
      throw ValidationError(
        field: "photo",
        message:
          "This JPEG would not be smaller. Keep your original or choose another supported copy.")
    }
    return try await replace(
      DerivedPhotoSource(filename: "Smaller JPEG copy", file: file), accountId: accountId,
      photoId: photoId, maxBytes: maxBytes, publicConsent: publicConsent)
  }

  private func saveFile(
    _ file: URL, accountId: UUID, voyageId: UUID, maxBytes: Int64,
    pendingId: UUID, generation: Int
  ) async throws -> UUID {
    let state = try await store.read(accountId: accountId)
    guard state.account.access != .locked, state.account.generation == generation,
      let pending = state.photos.first(where: { $0.id == pendingId && !$0.deleted })
    else { throw WorkspaceError.locked }
    var photo = try await preparer.prepare(
      file, accountId: accountId, voyageId: voyageId,
      maxBytes: maxBytes, photoId: pending.id, uploadKey: pending.uploadKey)
    photo.filename = pending.filename
    try await repository.addPhoto(
      accountId: accountId, photo: photo,
      publicConsent: pending.importPublicConsent == true, expectedGeneration: generation)
    return pending.id
  }
}

private struct DerivedPhotoSource: PhotoAcquiring {
  let filename: String
  let file: URL
  func acquire() -> URL { file }
}
