import XCTest

@MainActor
final class PublicBrowsingTests: UITestCase {
  func testPublicVoyageGalleryAndProfileUseTheCapturedAPIContract() throws {
    let app = try launchPublicApp()
    let card = app.staticTexts["Three weeks in Hokkaido"].firstMatch
    XCTAssertTrue(card.waitUntilExists(timeout: 10))
    card.tap()
    XCTAssertTrue(app.buttons["View traveler"].waitUntilExists(timeout: 10), app.debugDescription)
    app.buttons["Photos"].tap()
    let photo = app.buttons["Photo 2, no recorded location"].firstMatch
    XCTAssertTrue(photo.waitUntilExists(timeout: 5), app.debugDescription)
    photo.tap()
    XCTAssertTrue(app.buttons["Info"].waitUntilExists(timeout: 5))
    assertGalleryFillsWindow(app, context: "public photos")
    app.buttons["Info"].tap()
    XCTAssertTrue(app.staticTexts["No recorded location"].waitUntilExists(timeout: 5))
    app.buttons["Done"].tap()
    app.buttons["Next"].tap()
    app.buttons["Previous"].tap()
    app.buttons["Close"].tap()
    app.buttons["View traveler"].tap()
    XCTAssertTrue(app.staticTexts["fixture-owner"].waitUntilExists(timeout: 5))
    XCTAssertTrue(app.staticTexts["Public profile"].exists)
  }

  func testDiscoverSetupReturnsToRecentFromEveryTabAndANestedProfile() throws {
    let app = try launchPublicApp()
    let card = app.buttons.matching(
      NSPredicate(format: "label BEGINSWITH %@", "Three weeks in Hokkaido")
    ).firstMatch
    for title in ["My Voyages", "Sync", "Account"] {
      app.tabBars.buttons[title].tap()
      try app.showDiscover()
      let root = app.navigationBars["Discover"]
      XCTAssertTrue(root.waitUntilExists(timeout: 5), "Discover setup must leave \(title).")
      guard root.exists else { return }
      XCTAssertTrue(card.waitUntilExists(timeout: 5))
      XCTAssertTrue(card.waitUntilHittable(timeout: 5))
    }

    app.buttons["Map"].tap()
    try app.showDiscover()
    XCTAssertTrue(card.waitUntilHittable(timeout: 5), "Discover setup must select Recent.")
    card.tap()
    let traveler = app.buttons["View traveler"]
    XCTAssertTrue(traveler.waitUntilExists(timeout: 5))
    traveler.tap()
    XCTAssertTrue(app.staticTexts["fixture-owner"].waitUntilExists(timeout: 5))
    app.tabBars.buttons["My Voyages"].tap()
    try app.showDiscover()
    XCTAssertTrue(app.navigationBars["Discover"].waitUntilExists(timeout: 5))
    XCTAssertTrue(
      card.waitUntilHittable(timeout: 5),
      "Discover setup must leave the nested profile and voyage."
    )

    card.tap()
    app.buttons["View traveler"].tap()
    app.tabBars.buttons["Account"].tap()
    app.terminate()
    app.launchEnvironment["PINVOYAGER_TEST_TAB"] = "discover"
    app.launch()
    XCTAssertTrue(app.navigationBars["Discover"].waitUntilExists(timeout: 5))
    XCTAssertTrue(
      card.waitUntilHittable(timeout: 5),
      "Fixture launches must ignore the previous tab and public path."
    )
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
        (JSONSerialization.jsonObject(with: list) as? [String: Any])?["voyages"]
      ),
      "next_before": NSNull(),
    ])
    reply("/api/profiles/" + ownerId, profile)
    let image = try XCTUnwrap(
      Data(
        base64Encoded:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLItAAAAABJRU5ErkJggg=="
      )
    )
    for photo in photos {
      for key in ["thumbnail_url", "display_url", "full_url"] {
        reply(try XCTUnwrap(photo[key] as? String), image)
      }
    }
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"]
    app.launchEnvironment["PINVOYAGER_TEST_RESPONSES"] = String(
      decoding: try JSONSerialization.data(withJSONObject: replies),
      as: UTF8.self
    )
    app.launchFixture()
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
    XCTAssertTrue(app.staticTexts["Approximate location"].waitUntilExists(timeout: 5))
    app.reveal(app.buttons["Show on map"])
    app.buttons["Show on map"].tap()
    XCTAssertTrue(app.buttons["Fit voyage"].waitUntilExists(timeout: 5))
    let pin = app.buttons.matching(
      NSPredicate(
        format: "label BEGINSWITH %@ AND label ENDSWITH %@",
        "View ",
        "photographs at this location"
      )
    ).firstMatch
    app.reveal(pin, swipeUp: false)
    XCTAssertTrue(pin.waitUntilExists(timeout: 10))
    pin.tap()
    assertGalleryFillsWindow(app, context: "public map")
    app.buttons["Close"].tap()
    app.reveal(photo)
    photo.tap()
    app.buttons["Info"].tap()
    app.reveal(app.buttons["Report photo"])
    app.buttons["Report photo"].tap()
    XCTAssertTrue(app.buttons["Sign in to submit"].waitUntilExists(timeout: 5))
    app.buttons["Sign in to submit"].tap()
    XCTAssertTrue(app.buttons["Refresh sign-in options"].waitUntilExists(timeout: 5))
    app.buttons["Cancel"].tap()
    XCTAssertTrue(app.buttons["View traveler"].waitUntilExists(timeout: 5))
  }
}
