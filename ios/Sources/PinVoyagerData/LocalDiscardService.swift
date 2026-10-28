import Foundation
import PinVoyagerDomain

/// A user-confirmed discard cancels transfers before releasing files; receipt requests remain intact.
public struct LocalDiscardService: Sendable {
  private let store: any WorkspaceStore
  private let vault: FileVault
  private let cancelTransfers: @Sendable (UUID) async -> Void
  public init(
    store: any WorkspaceStore, vault: FileVault,
    cancelTransfers: @escaping @Sendable (UUID) async -> Void
  ) {
    self.store = store
    self.vault = vault
    self.cancelTransfers = cancelTransfers
  }
  public func deleteLocalOnly(accountId: UUID, voyageId: UUID) async throws {
    try await discard(accountId: accountId, voyageId: voyageId, unavailable: false)
  }
  public func discardUnavailable(accountId: UUID, voyageId: UUID) async throws {
    try await discard(accountId: accountId, voyageId: voyageId, unavailable: true)
  }
  private func discard(accountId: UUID, voyageId: UUID, unavailable: Bool) async throws {
    let initial = try await store.read(accountId: accountId)
    try await store.update(accountId: accountId) { current in
      guard current.account.access != .locked,
        current.account.generation == initial.account.generation
      else { throw WorkspaceError.locked }
      guard let index = current.voyages.firstIndex(where: { $0.id == voyageId }) else {
        throw WorkspaceError.missingVoyage
      }
      guard
        !current.publicationIntents.contains(where: {
          $0.voyageId == voyageId && current.publicationMayProceed($0)
        })
      else { throw WorkspaceError.publicationArmed }
      let operations = current.operations.filter { $0.voyageId == voyageId }
      if unavailable {
        guard operations.contains(where: { $0.state == .conflict }) else {
          throw WorkspaceError.invalidOrder
        }
      } else {
        guard current.voyages[index].serverId == nil, operations.allSatisfy({ $0.request == nil })
        else {
          throw ValidationError(
            field: "voyage",
            message:
              "This voyage may exist on the website. Check its server outcome before deleting only its local files."
          )
        }
      }
      current.voyages[index].deleted = true
      for index in current.photos.indices where current.photos[index].voyageId == voyageId {
        current.photos[index].deleted = true
      }
      for index in current.operations.indices
      where current.operations[index].voyageId == voyageId
        && current.operations[index].state != .applied
      {
        current.operations[index].state = .canceled
      }
      for index in current.publicationIntents.indices
      where current.publicationIntents[index].voyageId == voyageId {
        current.publicationIntents[index].status = .needsReview
      }
    }
    await cancelTransfers(accountId)
    let tombstone = try await store.read(accountId: accountId)
    let operationIds = Set(tombstone.operations.filter { $0.voyageId == voyageId }.map(\.id))
    let candidates = Self.files(tombstone.photos.filter { $0.voyageId == voyageId })
      .union(tombstone.transfers.filter { operationIds.contains($0.operationId) }.map(\.bodyPath))
    try await store.update(accountId: accountId) { current in
      guard current.account.access != .locked,
        current.account.generation == initial.account.generation,
        current.voyages.contains(where: { $0.id == voyageId && $0.deleted })
      else { throw WorkspaceError.locked }
      for index in current.photos.indices where current.photos[index].voyageId == voyageId {
        current.photos[index].source = nil
        current.photos[index].previewPath = nil
        current.photos[index].offlineCopy = nil
      }
      current.transfers.removeAll { operationIds.contains($0.operationId) }
      current.pendingFileRemovals = Array(Set(current.pendingFileRemovals ?? []).union(candidates))
        .sorted()
      current.workingCopies.removeAll { $0.voyageId == voyageId }
      current.conflictReviews?.removeAll { $0.voyageId == voyageId }
    }
    try await finishPendingRemovals(accountId: accountId)
  }
  public func finishPendingRemovals(accountId: UUID) async throws {
    let initial = try await store.read(accountId: accountId)
    guard initial.account.access != .locked else { throw WorkspaceError.locked }
    for path in initial.pendingFileRemovals ?? [] {
      let current = try await store.read(accountId: accountId)
      guard current.account.access != .locked,
        current.account.generation == initial.account.generation
      else { throw WorkspaceError.locked }
      let retained = Self.files(current.photos).union(current.transfers.map(\.bodyPath))
      if !retained.contains(path) { try await vault.remove(path, accountId: accountId) }
      try await store.update(accountId: accountId) {
        $0.pendingFileRemovals?.removeAll { $0 == path }
      }
    }
  }
  private static func files(_ photos: [LocalPhoto]) -> Set<String> {
    Set(
      photos.flatMap { photo in
        [photo.source?.relativePath, photo.previewPath].compactMap { $0 }
          + Array(photo.offlineCopy?.variants.values ?? [:].values)
      })
  }
}
