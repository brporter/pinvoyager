import Foundation
import PinVoyagerDomain

public protocol PublicationNotifications: Sendable {
  func requestPermission() async throws -> Bool
  func contains(id: String) async throws -> Bool
  func send(id: String, title: String) async throws
  func remove(id: String) async
}

/// Notifications follow confirmed server state. They never establish publication.
public actor PublicationNotifier {
  private let store: any WorkspaceStore
  private let notifications: any PublicationNotifications
  private var notifying: Set<UUID> = []
  public init(store: any WorkspaceStore, notifications: any PublicationNotifications) {
    self.store = store
    self.notifications = notifications
  }

  public func setEnabled(_ enabled: Bool, accountId: UUID) async throws {
    let state = try await store.read(accountId: accountId)
    guard state.account.access != .locked else { throw WorkspaceError.locked }
    let allowed = enabled ? try await notifications.requestPermission() : false
    try await store.update(accountId: accountId) { current in
      guard current.account.generation == state.account.generation,
        current.account.access != .locked
      else { throw WorkspaceError.locked }
      current.policy.notifyOnPublication = allowed
    }
    if enabled && !allowed {
      throw ValidationError(
        field: "notifications",
        message: "Notification access is off. Enable it in Settings to receive publication updates."
      )
    }
  }

  public static func identifier(accountId: UUID, intentId: UUID) -> String {
    "publication.\(accountId).\(intentId)"
  }

  public func notifyCompleted(accountId: UUID, now: Date = Date()) async throws {
    guard !notifying.contains(accountId) else { return }
    notifying.insert(accountId)
    defer { notifying.remove(accountId) }
    let state = try await store.read(accountId: accountId)
    guard state.account.access == .unlocked, state.policy.notifyOnPublication == true else {
      return
    }
    for intent in state.publicationIntents
    where intent.status == .published && intent.notifiedAt == nil {
      guard
        state.voyages.contains(where: {
          $0.id == intent.voyageId && !$0.deleted && $0.confirmedPublished
        })
      else { continue }
      let id = Self.identifier(accountId: accountId, intentId: intent.id)
      if try await !notifications.contains(id: id) {
        try await notifications.send(id: id, title: intent.fields.title)
      }
      do {
        try await store.update(accountId: accountId) { current in
          guard current.account.generation == state.account.generation,
            current.account.access == .unlocked, current.policy.notifyOnPublication == true,
            current.voyages.contains(where: {
              $0.id == intent.voyageId && !$0.deleted && $0.confirmedPublished
            }),
            let index = current.publicationIntents.firstIndex(where: {
              $0.id == intent.id && $0.status == .published
            })
          else {
            throw WorkspaceError.locked
          }
          current.publicationIntents[index].notifiedAt = now
        }
      } catch WorkspaceError.locked {
        await notifications.remove(id: id)
        return
      }
    }
  }
}
