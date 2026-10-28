import Foundation
import Observation
import PinVoyagerData
import PinVoyagerDomain

@MainActor @Observable final class ProfilePresentation {
  private let identifier: String
  private let client: APIClient
  private var requestId = UUID()
  private(set) var profile: PublicProfile?
  private(set) var voyages: [VoyageSummary] = []
  private(set) var cursor: String?
  private(set) var loadedAccess: PublicAccess?
  private(set) var error: String?
  private(set) var loading = false
  private(set) var retryingMore = false
  init(identifier: String, client: APIClient) {
    self.identifier = identifier
    self.client = client
  }
  func invalidate() {
    requestId = UUID()
    profile = nil
    voyages = []
    cursor = nil
    error = nil
    loadedAccess = nil
    loading = false
  }
  func load(refresh: Bool, access: PublicAccess, isCurrent: @MainActor (PublicAccess) -> Bool) async
  {
    guard !access.offline else {
      invalidate()
      error = "Public profiles are not available offline."
      return
    }
    guard !loading else { return }
    let id = UUID()
    requestId = id
    loading = true
    retryingMore = !refresh
    defer { if requestId == id { loading = false } }
    do {
      let page = try await client.profile(
        identifier, before: refresh ? nil : cursor, accountId: access.accountId)
      guard requestId == id, isCurrent(access), !Task.isCancelled else { return }
      loadedAccess = access
      var seen: Set<UUID> = []
      voyages = ((refresh ? [] : voyages) + page.voyages).filter { seen.insert($0.id).inserted }
      cursor = page.nextBefore
      profile = page
      error = nil
    } catch {
      guard requestId == id, isCurrent(access), !Task.isCancelled else { return }
      if let denied = error as? APIError, [401, 403, 404].contains(denied.status) {
        profile = nil
        voyages = []
        cursor = nil
      }
      self.error = error.localizedDescription
    }
  }
  func retry(access: PublicAccess, isCurrent: @MainActor (PublicAccess) -> Bool) async {
    await load(refresh: !retryingMore, access: access, isCurrent: isCurrent)
  }
}
