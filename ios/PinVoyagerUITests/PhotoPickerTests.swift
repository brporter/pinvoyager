import XCTest

@MainActor
final class PhotoPickerTests: UITestCase {
  func testSystemPickerCopiesTheSelectedPhotoAndKeepsItsBatchCellularPermission() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing",
      "--signed-in-fixture",
      "--populated-fixture",
      "--offline-fixture",
    ]
    app.launchFixture(tab: .voyages)
    app.staticTexts["Winter in Hokkaido"].tap()
    app.reveal(app.buttons["Add photos"])
    app.buttons["Add photos"].tap()
    let cellular = app.switches["Allow cellular for this import only"]
    XCTAssertTrue(cellular.waitUntilExists(timeout: 5))
    cellular.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    app.buttons["Choose photos"].tap()
    app.selectFirstSystemPhoto()
    XCTAssertTrue(app.staticTexts["Photo 1"].waitUntilExists(timeout: 10), app.debugDescription)
    XCTAssertTrue(app.navigationBars["Photos"].waitForNonExistence(timeout: 10))
    let reset = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "enabled == true AND value == %@", "0"),
      object: cellular
    )
    XCTAssertEqual(
      XCTWaiter.wait(for: [reset], timeout: 30),
      .completed,
      "The import must finish and require fresh cellular permission for a later batch. \(app.debugDescription)"
    )
    XCTAssertTrue(
      app.navigationBars["Add photos"].buttons["Close"].wait(
        for: \.isEnabled,
        toEqual: true,
        timeout: 10
      )
    )
    app.navigationBars["Add photos"].buttons["Close"].tap()
    app.buttons["Sync"].tap()
    app.buttons["Sync and storage settings"].tap()
    let exception = app.switches.matching(
      NSPredicate(format: "label BEGINSWITH %@", "Use cellular for this batch")
    ).firstMatch
    XCTAssertTrue(exception.waitUntilExists(timeout: 5))
    XCTAssertEqual(exception.value as? String, "1")
    XCTAssertEqual(app.switches["Upload photos on Wi-Fi only"].value as? String, "1")
  }
}
