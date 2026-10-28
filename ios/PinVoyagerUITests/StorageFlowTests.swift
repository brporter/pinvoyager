import PinVoyagerDomain
import UIKit
import XCTest

@MainActor final class StorageFlowTests: XCTestCase {
  func testDownloadManagerSavesAndRemovesAnOwnerCopyAndExportListsMissingOriginals() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Offline download management", description: "A website voyage"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Unavailable original.jpg")
    photo.serverId = UUID()
    photo.transfer = .accepted
    photo.variant = .ready
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 120, height: 80)).jpegData(
      withCompressionQuality: 0.8
    ) {
      UIColor.systemOrange.setFill()
      $0.fill(CGRect(x: 0, y: 0, width: 120, height: 80))
    }
    try fixture.provideOwnerDownload(voyage: voyage, photo: photo, bytes: bytes)
    let app = try fixture.launch()
    app.buttons["Account"].tap()
    app.buttons["Sync and storage"].tap()
    app.reveal(app.buttons["Manage downloads"])
    app.buttons["Manage downloads"].tap()
    XCTAssertTrue(
      app.staticTexts["0 of 1 photos saved for offline use"].waitForExistence(timeout: 5))
    app.buttons["Download or resume owner copy"].tap()
    XCTAssertTrue(
      app.staticTexts["1 of 1 photos saved for offline use"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Last saved")).firstMatch
        .exists)
    app.buttons["Remove download"].tap()
    try XCTUnwrap(
      app.buttons.matching(identifier: "Remove download").allElementsBoundByIndex.last {
        $0.isHittable
      }
    ).tap()
    XCTAssertTrue(
      app.staticTexts["0 of 1 photos saved for offline use"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.reveal(app.buttons["Export local draft"])
    app.buttons["Export local draft"].tap()
    app.staticTexts[voyage.fields.title].tap()
    XCTAssertTrue(
      app.staticTexts["Originals unavailable on this device"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts[photo.filename].exists)
    app.buttons["Prepare export of local work"].tap()
    XCTAssertTrue(app.buttons["Save export to Files"].waitForExistence(timeout: 5))
  }
}
