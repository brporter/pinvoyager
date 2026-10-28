import Foundation

public struct UserProfile: Codable, Equatable, Identifiable, Sendable {
  public let id: UUID
  public var displayName: String?
  public var avatarUrl: String?
  public var createdAt: String?
  public var isSiteAdmin: Bool?
  public init(id: UUID, displayName: String? = nil) {
    self.id = id
    self.displayName = displayName
  }
}

public enum AccountAccess: String, Codable, Sendable { case unlocked, requiresSignIn, locked }

public struct LocalAccount: Codable, Equatable, Identifiable, Sendable {
  public var id: UUID { profile.id }
  public var profile: UserProfile
  public var generation: Int = 0
  public var access: AccountAccess = .unlocked
  public var verifiedAt: Date
  public init(profile: UserProfile, verifiedAt: Date = Date()) {
    self.profile = profile
    self.verifiedAt = verifiedAt
  }
}

public struct Workspace: Codable, Equatable, Sendable {
  public var account: LocalAccount
  public var voyages: [LocalVoyage] = []
  public var photos: [LocalPhoto] = []
  public var operations: [OutboxOperation] = []
  public var conflictReviews: [ConflictReview]?
  public var workingCopies: [VoyageWorkingCopy] = []
  public var publicationIntents: [PublicationIntent] = []
  public var transfers: [BackgroundTransfer] = []
  public var blockedAccounts: Set<UUID> = []
  public var policy = SyncPolicy()
  public var pendingFileRemovals: [String]?
  public var scratch: VoyageFields?
  public init(account: LocalAccount) { self.account = account }
  public func photos(for voyageId: UUID) -> [LocalPhoto] {
    photos.filter { $0.voyageId == voyageId && !$0.deleted }.sorted {
      ($0.order, $0.id.uuidString) < ($1.order, $1.id.uuidString)
    }
  }
  public func uploadPolicy(for photoId: UUID) -> SyncPolicy {
    let photo = photos.first { $0.id == photoId }
    var result = policy.forUpload(cellularException: photo?.cellularException == true)
    if voyages.first(where: { $0.id == photo?.voyageId })?.uploadsPaused == true {
      result.paused = true
    }
    return result
  }
  public func publicationMayProceed(_ intent: PublicationIntent) -> Bool {
    intent.isArmed
      || (intent.serverId == nil && intent.status != .published
        && operations.contains {
          if case .publish(let id) = $0.kind {
            return id == intent.id && $0.request != nil && $0.receiptRejected != true
          }
          return false
        })
  }
  public func canUndoPhotoRemoval(_ ids: Set<UUID>) -> Bool {
    !ids.isEmpty
      && ids.allSatisfy { id in
        guard photos.contains(where: { $0.id == id && $0.deleted }),
          let removal = operations.last(where: {
            if case .removePhoto(let photo, _) = $0.kind { photo == id } else { false }
          })
        else { return false }
        return removal.state == .waiting && removal.request == nil
      }
  }
  public func canUndoVoyageDeletion(_ id: UUID) -> Bool {
    guard voyages.contains(where: { $0.id == id && $0.deleted && $0.deletionUndo != nil }),
      let deletion = operations.last(where: { $0.voyageId == id && $0.kind == .deleteVoyage })
    else { return false }
    return deletion.state == .waiting && deletion.request == nil
  }
  public var attentionCount: Int {
    let pending = operations.filter { [.attention, .conflict].contains($0.state) }
    let uploadPhotos = Set(
      pending.compactMap { operation -> UUID? in
        if case .upload(let id, _) = operation.kind { return id }
        return nil
      })
    return pending.count
      + photos.filter {
        !$0.deleted && !uploadPhotos.contains($0.id)
          && ($0.variant == .failed || [.needsSource, .needsAttention].contains($0.transfer))
      }.count
  }
}

public enum WorkspaceError: Error, Equatable, LocalizedError, Sendable {
  case missing, locked, wrongAccount, missingVoyage, invalidOrder, publicationArmed,
    unsupportedProtocol, malformedReceipt
  public var errorDescription: String? {
    switch self {
    case .missing: "The saved workspace could not be found."
    case .locked: "Sign in to this account to unlock its saved voyages."
    case .wrongAccount: "These drafts belong to another account."
    case .missingVoyage: "This voyage is unavailable. Your saved work has been kept."
    case .invalidOrder: "The order must include every photograph exactly once."
    case .publicationArmed: "Cancel the scheduled publication online before changing this voyage."
    case .unsupportedProtocol:
      "This server does not yet support native synchronization. Saved work remains on this device."
    case .malformedReceipt:
      "The server outcome is uncertain. Checking its receipt is required before retrying."
    }
  }
}

public protocol WorkspaceStore: Sendable {
  func accountIds() async throws -> [UUID]
  func insert(_ workspace: Workspace) async throws
  func read(accountId: UUID) async throws -> Workspace
  func update(accountId: UUID, _ change: @escaping @Sendable (inout Workspace) throws -> Void)
    async throws
  func remove(accountId: UUID) async throws
}
