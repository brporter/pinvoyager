import Foundation
import PinVoyagerDomain

public struct OwnerVoyageSnapshot: Sendable {
  public let voyage: RemoteVoyage
  public let photos: [OwnerPhoto]
}

public protocol OwnerSnapshotLoading: Sendable {
  func ownerSnapshot(voyageId: UUID, accountId: UUID) async throws -> OwnerVoyageSnapshot
}

extension APIClient: OwnerSnapshotLoading {
  public func ownerPhotos(voyageId: UUID, accountId: UUID) async throws -> [OwnerPhoto] {
    var photos: [OwnerPhoto] = []
    var cursor: String?
    var visited: Set<String> = []
    repeat {
      let page: OwnerPhotoPage = try await get(
        path: "/api/voyages/\(voyageId)/photos",
        query: cursor.map { [URLQueryItem(name: "before", value: $0)] } ?? [],
        accountId: accountId)
      photos.append(contentsOf: page.photos)
      cursor = page.nextBefore
      if let cursor, !visited.insert(cursor).inserted { throw URLError(.badServerResponse) }
    } while cursor != nil
    return photos
  }

  /// A second revision read detects content changes during private metadata pagination.
  public func ownerSnapshot(voyageId: UUID, accountId: UUID) async throws -> OwnerVoyageSnapshot {
    let before = try await voyage(voyageId, accountId: accountId)
    guard before.ownerId == accountId else { throw WorkspaceError.wrongAccount }
    let photos = try await ownerPhotos(voyageId: voyageId, accountId: accountId)
    let after = try await voyage(voyageId, accountId: accountId)
    guard before.revision != nil, before.revision == after.revision,
      after.ownerId == accountId, photos.count == after.photos.count,
      Set(photos.map(\.id)).count == photos.count,
      Set(photos.map(\.id)) == Set(after.photos.map(\.id))
    else {
      throw ValidationError(
        field: "conflict",
        message:
          "The website changed during review. Reload its latest version before resolving changes.")
    }
    return OwnerVoyageSnapshot(voyage: after, photos: photos)
  }
}
