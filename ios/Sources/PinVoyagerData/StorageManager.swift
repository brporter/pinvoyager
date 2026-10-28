import CryptoKit
import Foundation
import PinVoyagerDomain

public struct StorageBreakdown: Equatable, Sendable {
  public init() {}
  public var unsyncedPhotos: Int64 = 0
  public var keptOriginals: Int64 = 0
  public var offlineVoyages: Int64 = 0
  public var temporaryImageCache: Int64 = 0
}

/// Explicit downloads and pending source files never participate in cache eviction.
public actor StorageManager {
  private let store: any WorkspaceStore
  private let vault: FileVault
  private let cache: (any ImageCaching)?
  public init(store: any WorkspaceStore, vault: FileVault, cache: (any ImageCaching)? = nil) {
    self.store = store
    self.vault = vault
    self.cache = cache
  }
  private static func accepted(_ photo: LocalPhoto, in state: Workspace) -> Bool {
    !photo.deleted && photo.serverId != nil && photo.transfer == .accepted
      && photo.variant == .ready
      && !state.operations.contains {
        $0.voyageId == photo.voyageId && ![.applied, .canceled].contains($0.state)
      }
  }
  public func breakdown(accountId: UUID) async throws -> StorageBreakdown {
    let state = try await store.read(accountId: accountId)
    guard state.account.access != .locked else { throw WorkspaceError.locked }
    var result = StorageBreakdown()
    let sources = Dictionary(
      grouping: state.photos.filter { $0.source != nil }, by: { $0.source!.relativePath })
    for (path, photos) in sources {
      let size = try await fileSize(path, accountId: accountId)
      if photos.allSatisfy({ Self.accepted($0, in: state) }) {
        result.keptOriginals += size
      } else {
        result.unsyncedPhotos += size
      }
    }
    let downloads = Set(
      state.photos.flatMap { Array($0.offlineCopy?.variants.values ?? [:].values) })
    for path in downloads {
      result.offlineVoyages += try await fileSize(path, accountId: accountId)
    }
    result.temporaryImageCache = try await cache?.byteCount() ?? 0
    return result
  }
  private func fileSize(_ path: String, accountId: UUID) async throws -> Int64 {
    let url = try await vault.url(path, accountId: accountId)
    guard FileManager.default.fileExists(atPath: url.path) else { return 0 }
    return Int64(try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
  }
  private func verifiedDisplay(_ photo: LocalPhoto, accountId: UUID) async throws -> Bool {
    guard let copy = photo.offlineCopy, let path = copy.variants["display"],
      path.hasPrefix("downloads/"),
      let hash = path.split(separator: "-").last?.split(separator: ".").first,
      hash.count == 64
    else { return false }
    let url = try await vault.url(path, accountId: accountId)
    guard let handle = try? FileHandle(forReadingFrom: url) else { return false }
    defer { try? handle.close() }
    var digest = SHA256()
    while let bytes = try handle.read(upToCount: 1_024 * 1_024), !bytes.isEmpty {
      digest.update(data: bytes)
    }
    return digest.finalize().map { String(format: "%02x", $0) }.joined() == hash
  }
  @discardableResult public func removeUploadedOriginals(accountId: UUID) async throws -> Int64 {
    let initial = try await store.read(accountId: accountId)
    guard initial.account.access == .unlocked else { throw WorkspaceError.locked }
    let groups = Dictionary(
      grouping: initial.photos.filter { $0.source != nil }, by: { $0.source!.relativePath })
    var eligible: Set<String> = []
    for (path, photos) in groups where photos.allSatisfy({ Self.accepted($0, in: initial) }) {
      var verified = true
      for photo in photos {
        if try await !verifiedDisplay(photo, accountId: accountId) {
          verified = false
          break
        }
      }
      if verified { eligible.insert(path) }
    }
    let paths = eligible
    let generation = initial.account.generation
    try await store.update(accountId: accountId) { current in
      guard current.account.access == .unlocked, current.account.generation == generation else {
        throw WorkspaceError.locked
      }
      for path in paths {
        let references = current.photos.filter { $0.source?.relativePath == path }
        guard !references.isEmpty,
          references.allSatisfy({ photo in
            Self.accepted(photo, in: current) && initial.photos.contains { $0 == photo }
          })
        else { throw WorkspaceError.locked }
      }
      for index in current.photos.indices
      where current.photos[index].source.map({ paths.contains($0.relativePath) }) == true {
        current.photos[index].source = nil
      }
    }
    var removed: Int64 = 0
    for path in paths {
      removed += try await fileSize(path, accountId: accountId)
      try await vault.remove(path, accountId: accountId)
    }
    return removed
  }
}
