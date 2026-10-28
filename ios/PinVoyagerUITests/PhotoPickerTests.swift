import XCTest

@MainActor final class PhotoPickerTests: XCTestCase {
  func testSystemPickerCopiesTheSelectedPhotoAndKeepsItsBatchCellularPermission() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
    ]
    app.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts["Winter in Hokkaido"].tap()
    app.reveal(app.buttons["Add photos"])
    app.buttons["Add photos"].tap()
    let cellular = app.switches["Allow cellular for this import only"]
    XCTAssertTrue(cellular.waitForExistence(timeout: 5))
    cellular.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    app.buttons["Choose photos"].tap()
    app.selectFirstSystemPhoto()
    XCTAssertTrue(app.staticTexts["Photo 1"].waitForExistence(timeout: 10), app.debugDescription)
    XCTAssertEqual(
      cellular.value as? String, "0", "A later import must require fresh cellular permission")
    app.navigationBars["Add photos"].buttons["Close"].tap()
    app.buttons["Sync"].tap()
    app.buttons["Sync and storage settings"].tap()
    let exception = app.switches.matching(
      NSPredicate(format: "label BEGINSWITH %@", "Use cellular for this batch")
    ).firstMatch
    XCTAssertTrue(exception.waitForExistence(timeout: 5))
    XCTAssertEqual(exception.value as? String, "1")
    XCTAssertEqual(app.switches["Upload photos on Wi-Fi only"].value as? String, "1")
  }
}
