import PinVoyagerDomain
import XCTest

@MainActor final class PhotoManagementTests: XCTestCase {
  func testStagedOrderCanBeCanceledAndSavedBeforeBulkRemoval() throws {
    var fixture = try FixtureLaunch()
    let voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Arrange photographs", description: "Keep the selected order"))
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = ["First", "Second", "Third"].enumerated().map { index, name in
      var photo = LocalPhoto(voyageId: voyage.id, filename: name)
      photo.order = index
      return photo
    }
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    let manage = app.buttons["Manage photos"]
    app.reveal(manage)
    XCTAssertTrue(manage.exists)
    guard manage.exists else { return }
    manage.tap()
    app.buttons["Reorder"].tap()
    app.buttons["Move Second earlier"].tap()
    XCTAssertTrue(app.buttons["Save order"].isEnabled)
    app.buttons["Cancel order changes"].tap()
    app.buttons["Reorder"].tap()
    XCTAssertFalse(app.buttons["Move First earlier"].isEnabled)
    app.buttons["Move First later"].tap()
    app.buttons["Save order"].tap()
    app.buttons["Done"].tap()
    manage.tap()
    app.buttons["Reorder"].tap()
    XCTAssertFalse(app.buttons["Move Second earlier"].isEnabled)
    app.buttons["Cancel order changes"].tap()
    app.buttons["Select"].tap()
    app.buttons["Select Second"].tap()
    app.buttons["Select Third"].tap()
    app.buttons["Remove selected"].tap()
    XCTAssertTrue(app.staticTexts["Remove 2 photographs?"].waitForExistence(timeout: 3))
    app.buttons["Remove 2 photographs"].tap()
    XCTAssertTrue(app.buttons["Select Second"].waitForNonExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Select First"].exists)
    app.buttons["Done selecting"].tap()
    try XCTUnwrap(
      app.buttons.matching(identifier: "Add photos").allElementsBoundByIndex.last { $0.isHittable }
    ).tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitForExistence(timeout: 5))
    app.navigationBars["Add photos"].buttons["Close"].tap()
    app.buttons["Done"].tap()
  }
}

extension PhotoManagementTests {
  func testBulkRemovalCanBeUndoneBeforeItsRequestIsSubmitted() throws {
    var fixture = try FixtureLaunch()
    let voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(
        title: "Undo selected removals", description: "Keep unsubmitted work reversible"))
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = ["One", "Two"].enumerated().map { index, name in
      var photo = LocalPhoto(voyageId: voyage.id, filename: name)
      photo.order = index
      return photo
    }
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    app.reveal(app.buttons["Manage photos"])
    app.buttons["Manage photos"].tap()
    app.buttons["Select"].tap()
    app.buttons["Select One"].tap()
    app.buttons["Select Two"].tap()
    app.buttons["Remove selected"].tap()
    app.buttons["Remove 2 photographs"].tap()
    let undo = app.buttons["Undo removals"]
    XCTAssertTrue(undo.waitForExistence(timeout: 5))
    guard undo.exists else { return }
    undo.tap()
    XCTAssertTrue(app.buttons["Select One"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Select Two"].exists)
  }
}
