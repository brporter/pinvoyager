import PinVoyagerDomain
import UserNotifications
import XCTest

@testable import PinVoyager

private actor NotificationCenterStub: PublicationNotificationCenter {
  var permission = true
  var authorization: UNAuthorizationStatus = .authorized
  var pending: [String] = []
  var delivered: [String] = []
  var requests: [PublicationNotice] = []
  var removed: [String] = []
  func requestPermission() -> Bool { permission }
  func pendingIds() -> [String] { pending }
  func deliveredIds() -> [String] { delivered }
  func status() -> UNAuthorizationStatus { authorization }
  func add(_ request: PublicationNotice) { requests.append(request) }
  func remove(_ id: String) {
    removed.append(id)
    pending.removeAll { $0 == id }
    delivered.removeAll { $0 == id }
  }
  func configure(
    permission: Bool, authorization: UNAuthorizationStatus, pending: [String], delivered: [String]
  ) {
    self.permission = permission
    self.authorization = authorization
    self.pending = pending
    self.delivered = delivered
  }
}

@MainActor final class PublicationNotificationAdapterTests: XCTestCase {
  func testSystemAdapterChecksPendingAndDeliveredIdentifiersAndUsesReviewedPublicTitles()
    async throws
  {
    let center = NotificationCenterStub()
    let notifications = SystemPublicationNotifications(center: center)
    let permission = try await notifications.requestPermission()
    XCTAssertTrue(permission)
    await center.configure(
      permission: false, authorization: .provisional, pending: ["pending"], delivered: ["delivered"]
    )
    let pending = await notifications.contains(id: "pending")
    let delivered = await notifications.contains(id: "delivered")
    let absent = await notifications.contains(id: "absent")
    XCTAssertTrue(pending)
    XCTAssertTrue(delivered)
    XCTAssertFalse(absent)
    try await notifications.send(id: "publication", title: "A reviewed public voyage")
    let savedRequests = await center.requests
    let request = try XCTUnwrap(savedRequests.first)
    XCTAssertEqual(request.identifier, "publication")
    XCTAssertEqual(request.title, "Voyage published")
    XCTAssertEqual(request.body, "A reviewed public voyage")
    await notifications.remove(id: "pending")
    let remaining = await notifications.contains(id: "pending")
    XCTAssertFalse(remaining)
    await center.configure(permission: false, authorization: .denied, pending: [], delivered: [])
    let denied = try await notifications.requestPermission()
    XCTAssertFalse(denied)
    do {
      try await notifications.send(id: "denied", title: "Unsent")
      XCTFail("Denied access must prevent delivery")
    } catch is ValidationError {} catch { XCTFail("Unexpected error: \(error)") }
    let requests = await center.requests
    XCTAssertEqual(requests.count, 1)
  }
}
