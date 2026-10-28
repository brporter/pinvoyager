import Foundation
import PinVoyagerDomain

struct RecoveredPhotoSource: Sendable {
  let original: LocalPhoto
  let source: SourceFile
  let isOriginal: Bool
}

/// Copies verified local material into independent files before creating a new private identity.
public struct DraftRecoveryService: Sendable {
  private let store: any WorkspaceStore
  private let vault: FileVault
  public init(store: any WorkspaceStore, vault: FileVault) {
    self.store = store
    self.vault = vault
  }
  public func recover(accountId: UUID, voyageId: UUID) async throws -> UUID {
    let initial = try await store.read(accountId: accountId)
    guard initial.account.access != .locked else { throw WorkspaceError.locked }
    guard initial.voyages.contains(where: { $0.id == voyageId }) else {
      throw WorkspaceError.missingVoyage
    }
    var recovered: [RecoveredPhotoSource] = []
    do {
      for photo in initial.photos(for: voyageId) {
        try Task.checkCancellation()
        if let source = photo.source,
          let copy = try await verifiedCopy(
            source.relativePath, hash: source.contentHash,
            byteCount: source.byteCount, accountId: accountId)
        {
          recovered.append(RecoveredPhotoSource(original: photo, source: copy, isOriginal: true))
          continue
        }
        for kind in ["full", "display"] {
          guard let path = photo.offlineCopy?.variants[kind], path.hasPrefix("downloads/"),
            let hash = path.split(separator: "-").last?.split(separator: ".").first,
            hash.count == 64,
            let copy = try await verifiedCopy(path, hash: String(hash), accountId: accountId)
          else { continue }
          recovered.append(RecoveredPhotoSource(original: photo, source: copy, isOriginal: false))
          break
        }
      }
      return try await LocalRepository(store: store).recoverAsNewDraft(
        accountId: accountId,
        voyageId: voyageId, sources: recovered, expectedGeneration: initial.account.generation)
    } catch {
      for photo in recovered {
        try? await vault.remove(photo.source.relativePath, accountId: accountId)
      }
      throw error
    }
  }
  private func verifiedCopy(_ path: String, hash: String, byteCount: Int64? = nil, accountId: UUID)
    async throws -> SourceFile?
  {
    let url = try await vault.url(path, accountId: accountId)
    guard FileManager.default.fileExists(atPath: url.path) else { return nil }
    let copy: SourceFile
    do {
      copy = try await vault.importFile(
        url, accountId: accountId, photoId: UUID(), maxBytes: 25 * 1_024 * 1_024)
    } catch FileVaultError.unsupportedFormat { return nil } catch FileVaultError.missingSource {
      return nil
    }
    guard copy.contentHash == hash, byteCount == nil || copy.byteCount == byteCount else {
      try await vault.remove(copy.relativePath, accountId: accountId)
      return nil
    }
    return copy
  }
}
