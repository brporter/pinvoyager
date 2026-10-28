import CryptoKit
import Foundation
import PinVoyagerDomain

public struct OfflineManifest: Decodable, Equatable, Sendable {
  public struct Photo: Decodable, Equatable, Sendable {
    public struct Variant: Decodable, Equatable, Sendable {
      public let kind: String
      public let url: String
      public let byteCount: Int64
      public let sha256: String
    }
    public let id: UUID
    public let position: Int
    public let width: Int
    public let height: Int
    public let capturedAt: String?
    public let latitude: Double?
    public let longitude: Double?
    public let variants: [Variant]
  }
  public let voyageId: UUID
  public let ownerId: UUID
  public let revision: String
  public let title: String
  public let description: String?
  public let granularity: Granularity
  public let published: Bool
  public let retentionPermission: String
  public let photos: [Photo]
}

/// Certifies an owner snapshot only after every file and a fresh manifest agree.
public actor OwnerDownloads {
  private let client: APIClient
  private let store: any WorkspaceStore
  private let vault: FileVault
  public init(client: APIClient, store: any WorkspaceStore, vault: FileVault) {
    self.client = client
    self.store = store
    self.vault = vault
  }
  public func downloadFull(accountId: UUID, photoId: UUID) async throws {
    let state = try await store.read(accountId: accountId)
    guard let photo = state.photos.first(where: { $0.id == photoId && !$0.deleted }),
      let remoteId = photo.serverId
    else { throw FileVaultError.missingSource }
    try await download(accountId: accountId, voyageId: photo.voyageId, fullPhotoId: remoteId)
  }
  public func download(accountId: UUID, voyageId: UUID) async throws {
    try await download(accountId: accountId, voyageId: voyageId, fullPhotoId: nil)
  }
  private func download(accountId: UUID, voyageId: UUID, fullPhotoId: UUID?) async throws {
    let initial = try await store.read(accountId: accountId)
    guard initial.account.access == .unlocked else { throw WorkspaceError.locked }
    guard let voyage = initial.voyages.first(where: { $0.id == voyageId && !$0.deleted }),
      let remoteId = voyage.serverId
    else { throw WorkspaceError.missingVoyage }
    guard
      !initial.operations.contains(where: {
        $0.voyageId == voyageId && ![.applied, .canceled].contains($0.state)
      })
    else {
      throw ValidationError(
        field: "download",
        message: "Synchronize this voyage before downloading its website version.")
    }
    let path = "/api/voyages/\(remoteId)/offline-manifest"
    let manifest: OfflineManifest = try await client.get(path: path, accountId: accountId)
    guard manifest.ownerId == accountId, manifest.voyageId == remoteId,
      manifest.retentionPermission == "owner_offline_copy_v1", manifest.photos.count <= 10_000,
      !manifest.revision.isEmpty, manifest.revision.allSatisfy(\.isNumber),
      Set(manifest.photos.map(\.id)).count == manifest.photos.count
    else { throw WorkspaceError.malformedReceipt }
    guard fullPhotoId == nil || manifest.photos.contains(where: { $0.id == fullPhotoId }) else {
      throw FileVaultError.missingSource
    }
    var copies: [UUID: OfflinePhotoCopy] = [:]
    for photo in manifest.photos {
      guard Set(photo.variants.map(\.kind)) == Set(["thumb", "display", "full"]),
        photo.variants.count == 3
      else { throw WorkspaceError.malformedReceipt }
      var files: [String: String] = [:]
      var size: Int64 = 0
      for variant in photo.variants {
        let prior = initial.photos.first { $0.voyageId == voyageId && $0.serverId == photo.id }?
          .offlineCopy
        let retainedFull = prior?.revision == manifest.revision && prior?.variants["full"] != nil
        guard variant.kind != "full" || photo.id == fullPhotoId || retainedFull else { continue }
        try Task.checkCancellation()
        let expected = "/api/photos/\(photo.id)/offline/\(variant.kind)"
        guard variant.url.lowercased() == expected.lowercased(), variant.byteCount > 0,
          variant.sha256.range(of: "^[a-f0-9]{64}$", options: .regularExpression) != nil
        else { throw WorkspaceError.malformedReceipt }
        let relative =
          "downloads/\(remoteId)/\(manifest.revision)/\(photo.id)/\(variant.kind)-\(variant.sha256).webp"
        let existing = try? await vault.read(relative, accountId: accountId)
        if existing.map({ Self.matches($0, variant) }) != true {
          let response = try await client.raw(method: "GET", path: expected, accountId: accountId)
          guard response.headers["x-pinvoyager-retention"] == "owner_offline_copy_v1",
            Self.matches(response.body, variant)
          else { throw WorkspaceError.malformedReceipt }
          try await vault.write(response.body, relativePath: relative, accountId: accountId)
        }
        files[variant.kind] = relative
        size += variant.byteCount
      }
      copies[photo.id] = OfflinePhotoCopy(
        revision: manifest.revision, variants: files, byteCount: size)
    }
    let current: OfflineManifest = try await client.get(path: path, accountId: accountId)
    guard current == manifest else { throw WorkspaceError.malformedReceipt }
    let completed = copies
    try await store.update(accountId: accountId) { state in
      guard state.account.access == .unlocked,
        state.account.generation == initial.account.generation,
        let index = state.voyages.firstIndex(where: { $0.id == voyageId && !$0.deleted }),
        !state.operations.contains(where: {
          $0.voyageId == voyageId && ![.applied, .canceled].contains($0.state)
        })
      else { throw WorkspaceError.locked }
      state.voyages[index].fields = VoyageFields(
        title: manifest.title, description: manifest.description, granularity: manifest.granularity)
      state.voyages[index].serverFields = state.voyages[index].fields
      state.voyages[index].revision = manifest.revision
      state.voyages[index].confirmedPublished = manifest.published
      for photo in manifest.photos {
        let index: Int
        if let existing = state.photos.firstIndex(where: {
          $0.voyageId == voyageId && $0.serverId == photo.id
        }) {
          index = existing
        } else {
          var local = LocalPhoto(voyageId: voyageId, filename: "Photo \(photo.position)")
          local.serverId = photo.id
          state.photos.append(local)
          index = state.photos.count - 1
        }
        state.photos[index].width = photo.width
        state.photos[index].height = photo.height
        state.photos[index].capturedAt = photo.capturedAt
        if let latitude = photo.latitude, let longitude = photo.longitude {
          let coordinate = Coordinate(latitude: latitude, longitude: longitude)
          if coordinate.isValid { state.photos[index].coordinate = coordinate }
        }
        state.photos[index].order = photo.position
        state.photos[index].variant = .ready
        state.photos[index].transfer = .accepted
        state.photos[index].offlineCopy = completed[photo.id]
      }
    }
    try await purgeMissing(
      accountId: accountId, voyageId: voyageId, keeping: Set(manifest.photos.map(\.id)),
      revision: manifest.revision)
  }
  public func purgeMissing(
    accountId: UUID, voyageId: UUID, keeping remoteIds: Set<UUID>, revision: String
  ) async throws {
    let initial = try await store.read(accountId: accountId)
    guard initial.account.access == .unlocked,
      initial.voyages.first(where: { $0.id == voyageId })?.revision == revision,
      !initial.operations.contains(where: {
        $0.voyageId == voyageId && ![.applied, .canceled].contains($0.state)
      })
    else { return }
    let removed = initial.photos.filter {
      $0.voyageId == voyageId && $0.serverId.map { !remoteIds.contains($0) } == true
    }
    let paths = removed.flatMap { Array($0.offlineCopy?.variants.values ?? [:].values) }
    let ids = Set(removed.map(\.id))
    try await store.update(accountId: accountId) { state in
      guard state.account.access == .unlocked,
        state.account.generation == initial.account.generation,
        state.voyages.first(where: { $0.id == voyageId })?.revision == revision,
        !state.operations.contains(where: {
          $0.voyageId == voyageId && ![.applied, .canceled].contains($0.state)
        })
      else { throw WorkspaceError.locked }
      for index in state.photos.indices where ids.contains(state.photos[index].id) {
        state.photos[index].offlineCopy = nil
        if state.photos[index].source == nil {
          state.photos[index].deleted = true
          state.photos[index].transfer = .removed
        } else {
          state.photos[index].transfer = .needsAttention
          state.photos[index].error =
            "This photograph was removed from the website. Your imported original has been kept."
        }
      }
    }
    for path in paths { try await vault.remove(path, accountId: accountId) }
  }

  public func remove(accountId: UUID, voyageId: UUID) async throws {
    let saved = try await store.read(accountId: accountId)
    let paths = saved.photos.filter { $0.voyageId == voyageId }.flatMap {
      Array($0.offlineCopy?.variants.values ?? [String: String]().values)
    }
    try await store.update(accountId: accountId) { state in
      for index in state.photos.indices where state.photos[index].voyageId == voyageId {
        state.photos[index].offlineCopy = nil
      }
    }
    for path in paths { try await vault.remove(path, accountId: accountId) }
  }
  private static func matches(_ bytes: Data, _ variant: OfflineManifest.Photo.Variant) -> Bool {
    Int64(bytes.count) == variant.byteCount
      && SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined() == variant.sha256
  }
}
