import OSLog
import PinVoyagerData
import PinVoyagerDomain
import UserNotifications

struct PublicationNotice: Sendable {
  let identifier: String
  let title: String
  let body: String
}

protocol PublicationNotificationCenter: Sendable {
  func requestPermission() async throws -> Bool

  func pendingIds() async -> [String]

  func deliveredIds() async -> [String]

  func status() async -> UNAuthorizationStatus

  func add(_ notice: PublicationNotice) async throws

  func remove(_ id: String) async
}

private actor SystemNotificationCenter: PublicationNotificationCenter {
  private let center = UNUserNotificationCenter.current()

  func requestPermission() async throws -> Bool {
    let logger = Logger(subsystem: "com.pinvoyager.app", category: "NotificationPermission")
    let initialStatus = await center.notificationSettings().authorizationStatus.rawValue
    logger.notice("Notification authorization requested; initial status \(initialStatus)")
    do {
      let allowed = try await center.requestAuthorization(options: [.alert, .sound])
      let finalStatus = await center.notificationSettings().authorizationStatus.rawValue
      logger.notice(
        "Notification authorization completed; allowed \(allowed), status \(finalStatus)"
      )
      return allowed
    } catch {
      logger.error(
        "Notification authorization failed: \(error.localizedDescription, privacy: .public)"
      )
      throw error
    }
  }

  func pendingIds() async -> [String] {
    await center.pendingNotificationRequests().map(\.identifier)
  }

  func deliveredIds() async -> [String] {
    await center.deliveredNotifications().map { $0.request.identifier }
  }

  func status() async -> UNAuthorizationStatus {
    await center.notificationSettings().authorizationStatus
  }

  func add(_ notice: PublicationNotice) async throws {
    let content = UNMutableNotificationContent()
    content.title = notice.title
    content.body = notice.body
    content.sound = .default
    try await center.add(
      UNNotificationRequest(identifier: notice.identifier, content: content, trigger: nil)
    )
  }

  func remove(_ id: String) {
    center.removePendingNotificationRequests(withIdentifiers: [id])
    center.removeDeliveredNotifications(withIdentifiers: [id])
  }
}

actor SystemPublicationNotifications: PublicationNotifications {
  private let center: any PublicationNotificationCenter

  init(center: any PublicationNotificationCenter = SystemNotificationCenter()) {
    self.center = center
  }

  func requestPermission() async throws -> Bool { try await center.requestPermission() }

  func contains(id: String) async -> Bool {
    if await center.pendingIds().contains(id) { return true }
    return await center.deliveredIds().contains(id)
  }

  func send(id: String, title: String) async throws {
    guard await [.authorized, .provisional, .ephemeral].contains(center.status()) else {
      throw ValidationError(
        field: "notifications",
        message: "Notification access is off. Enable it in Settings to receive publication updates."
      )
    }
    try await center.add(PublicationNotice(identifier: id, title: "Voyage published", body: title))
  }

  func remove(id: String) async { await center.remove(id) }
}
