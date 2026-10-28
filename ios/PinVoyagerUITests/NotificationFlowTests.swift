import PinVoyagerDomain
import XCTest

@MainActor final class NotificationFlowTests: XCTestCase {
  func testOptInAsksTheSystemAndConfirmedPublicationShowsAForegroundNotification() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Published notification check", description: "A reviewed voyage"))
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
      "/api/client-capabilities", body: try FixtureLaunch.fixture("native_capabilities"))
    let app = try fixture.launch()
    app.buttons["Account"].tap()
    app.buttons["Sync and storage"].tap()
    let optIn = app.switches["Notify when publication completes"]
    app.reveal(optIn)
    optIn.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let allow = springboard.alerts.buttons["Allow"]
    if allow.waitForExistence(timeout: 5) { allow.tap() }
    XCTAssertEqual(optIn.value as? String, "1", app.debugDescription)
    app.buttons["Sync"].tap()
    app.buttons["Sync now"].tap()
    XCTAssertTrue(
      springboard.staticTexts["Voyage published"].waitForExistence(timeout: 10),
      springboard.debugDescription)
  }
}
