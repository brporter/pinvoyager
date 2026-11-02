import PinVoyagerDomain
import XCTest

@MainActor
final class NotificationFlowTests: UITestCase {
  func testOptInAsksTheSystemAndConfirmedPublicationShowsAForegroundNotification() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Published notification check", description: "A reviewed voyage")
    )
    voyage.serverId = UUID()
    voyage.revision = "7"
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Reviewed photo")
    photo.serverId = UUID()
    photo.variant = .ready
    photo.transfer = .accepted
    var intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: Date())
    intent.serverId = UUID()
    intent.status = .published
    voyage.confirmedPublished = true
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    fixture.workspace.publicationIntents = [intent]
    fixture.reply(
      "/api/client-capabilities",
      body: try FixtureLaunch.fixture("native_capabilities")
    )
    let app = try fixture.launch(
      tab: .account,
      arguments: ["--notification-permission-diagnostics"]
    )
    app.buttons["Sync and storage"].tap()
    let optIn = app.switches["Notify when publication completes"]
    app.reveal(optIn)
    let authorization = app.staticTexts["notifications.initial-authorization"]
    XCTAssertTrue(authorization.waitUntilExists(timeout: 10), app.debugDescription)
    XCTAssertEqual(authorization.label, "Notification authorization: notDetermined")
    optIn.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let allow = springboard.alerts.buttons["Allow"]
    XCTAssertTrue(allow.waitUntilExists(timeout: 30), springboard.debugDescription)
    allow.tap()
    let enabled = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "value == %@", "1"),
      object: optIn
    )
    XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 15), .completed, app.debugDescription)
    app.buttons["Sync"].tap()
    app.buttons["Sync now"].tap()
    XCTAssertTrue(
      springboard.staticTexts["Voyage published"].waitUntilExists(timeout: 10),
      springboard.debugDescription
    )
  }
}
