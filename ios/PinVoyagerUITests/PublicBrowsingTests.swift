import XCTest

@MainActor final class PublicBrowsingTests: XCTestCase {
  func testPublicVoyageGalleryAndProfileUseTheCapturedAPIContract() throws {
    let app = try launchPublicApp()
    let card = app.staticTexts["Three weeks in Hokkaido"].firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()
    XCTAssertTrue(app.buttons["View traveler"].waitForExistence(timeout: 10), app.debugDescription)
    app.buttons["Photos"].tap()
    let photo = app.buttons["Photo 2, no recorded location"].firstMatch
    XCTAssertTrue(photo.waitForExistence(timeout: 5), app.debugDescription)
    photo.tap()
    XCTAssertTrue(app.buttons["Info"].waitForExistence(timeout: 5))
    assertGalleryFillsWindow(app, context: "public photos")
    app.buttons["Info"].tap()
    XCTAssertTrue(app.staticTexts["No recorded location"].waitForExistence(timeout: 5))
    app.buttons["Done"].tap()
    app.buttons["Next"].tap()
    app.buttons["Previous"].tap()
    app.buttons["Close"].tap()
    app.buttons["View traveler"].tap()
    XCTAssertTrue(app.staticTexts["fixture-owner"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Public profile"].exists)
  }
  private func launchPublicApp() throws -> XCUIApplication {
    let directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent()
      .appendingPathComponent("web/src/api/__fixtures__")
    func fixture(_ name: String) throws -> Data {
      try Data(contentsOf: directory.appendingPathComponent(name + ".json"))
    }
    let detail = try fixture("voyage_detail")
    let voyage = try XCTUnwrap(try JSONSerialization.jsonObject(with: detail) as? [String: Any])
    let voyageId = try XCTUnwrap(voyage["id"] as? String)
    let ownerId = try XCTUnwrap(voyage["owner_id"] as? String)
    let photos = try XCTUnwrap(voyage["photos"] as? [[String: Any]])
    let list = try fixture("voyage_list")
    var replies: [String: [String: String]] = [:]
    func reply(_ path: String, _ body: Data, status: Int = 200) {
      replies[path] = ["status": String(status), "body": body.base64EncodedString()]
    }
    reply("/api/voyages", list)
    reply("/api/voyages/" + voyageId, detail)
    let profile = try JSONSerialization.data(withJSONObject: [
      "user": ["id": ownerId, "display_name": "fixture-owner"],
      "voyages": try XCTUnwrap(
        (JSONSerialization.jsonObject(with: list) as? [String: Any])?["voyages"]),
      "next_before": NSNull(),
    ])
    reply("/api/profiles/" + ownerId, profile)
    let image = try XCTUnwrap(
      Data(
        base64Encoded:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLItAAAAABJRU5ErkJggg=="
      ))
    for photo in photos {
      for key in ["thumbnail_url", "display_url", "full_url"] {
        reply(try XCTUnwrap(photo[key] as? String), image)
      }
    }
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"]
    app.launchEnvironment["PINVOYAGER_TEST_RESPONSES"] = String(
      decoding: try JSONSerialization.data(withJSONObject: replies), as: UTF8.self)
    app.launch()
    return app
  }

}

extension PublicBrowsingTests {
  func testLocatedGalleryPhotoCanReturnToItsMapAndGuestReportingWaitsForSheetDismissal() throws {
    let app = try launchPublicApp()
    app.staticTexts["Three weeks in Hokkaido"].firstMatch.tap()
    app.buttons["Photos"].tap()
    let photo = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Photo 3"))
      .firstMatch
    app.reveal(photo)
    photo.tap()
    app.buttons["Info"].tap()
    XCTAssertTrue(app.staticTexts["Approximate location"].waitForExistence(timeout: 5))
    app.reveal(app.buttons["Show on map"])
    app.buttons["Show on map"].tap()
    XCTAssertTrue(app.buttons["Fit voyage"].waitForExistence(timeout: 5))
    let pin = app.buttons.matching(
      NSPredicate(
        format: "label BEGINSWITH %@ AND label ENDSWITH %@", "View ", "photographs at this location"
      )
    ).firstMatch
    app.reveal(pin, swipeUp: false)
    XCTAssertTrue(pin.waitForExistence(timeout: 10))
    pin.tap()
    assertGalleryFillsWindow(app, context: "public map")
    app.buttons["Close"].tap()
    app.reveal(photo)
    photo.tap()
    app.buttons["Info"].tap()
    app.reveal(app.buttons["Report photo"])
    app.buttons["Report photo"].tap()
    XCTAssertTrue(app.buttons["Sign in to submit"].waitForExistence(timeout: 5))
    app.buttons["Sign in to submit"].tap()
    XCTAssertTrue(app.buttons["Refresh sign-in options"].waitForExistence(timeout: 5))
    app.buttons["Cancel"].tap()
    XCTAssertTrue(app.buttons["View traveler"].waitForExistence(timeout: 5))
  }
}
