import PinVoyagerDomain
import XCTest

@MainActor final class SourceRecoveryTests: XCTestCase {
  func testReplacingAFailedPublicPhotoRequiresConsentAndKeepsExactlyOneActivePhoto() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(
        title: "Recover a failed photograph", description: "Replace only with consent"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    voyage.confirmedPublished = true
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Failed website photograph")
    photo.serverId = UUID()
    photo.transfer = .accepted
    photo.variant = .failed
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    let row = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", photo.filename))
      .firstMatch
    app.reveal(row)
    row.swipeLeft()
    app.buttons["Replace source"].tap()
    let choose = app.buttons["Choose a replacement source"]
    XCTAssertTrue(choose.waitForExistence(timeout: 5))
    XCTAssertFalse(choose.isEnabled)
    app.switches["The replacement will become public as it uploads"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)
    ).tap()
    choose.tap()
    app.selectFirstSystemPhoto(requiresConfirmation: false)
    XCTAssertTrue(
      app.navigationBars["Recover photograph"].waitForNonExistence(timeout: 10),
      app.debugDescription)
    XCTAssertTrue(row.waitForExistence(timeout: 5))
    XCTAssertTrue(row.label.contains("Saved on this iPhone"), row.label)
    app.buttons["Sync"].tap()
    app.reveal(app.staticTexts["Upload photograph"])
    XCTAssertTrue(app.staticTexts["Upload photograph"].exists)
    app.reveal(app.staticTexts["Remove photograph"], swipeUp: false)
    XCTAssertTrue(app.staticTexts["Remove photograph"].exists)
  }
}
