import XCTest

@MainActor final class ProfileFlowTests: XCTestCase {
  func testProfilePaginationReportingAndBlockingKeepTheirScope() throws {
    let fixture = try fixture()
    let app = try fixture.launch()
    app.buttons["View traveler"].firstMatch.tap()
    XCTAssertTrue(app.staticTexts["fixture-traveler"].waitForExistence(timeout: 5))
    app.reveal(app.buttons["Load more"])
    app.buttons["Load more"].tap()
    XCTAssertEqual(app.staticTexts.matching(identifier: "Three weeks in Hokkaido").count, 1)
    app.buttons["More"].tap()
    app.buttons["Report traveler"].tap()
    XCTAssertTrue(app.staticTexts["fixture-traveler"].waitForExistence(timeout: 5))
    app.buttons["Submit report"].tap()
    XCTAssertTrue(app.staticTexts["Waiting to send"].waitForExistence(timeout: 5))
    app.buttons["Done"].tap()
    app.buttons["More"].tap()
    app.buttons["Block traveler"].tap()
    XCTAssertTrue(app.staticTexts["Traveler blocked"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.staticTexts["Public profile"].exists)
  }

  func testGuestProfileBlockingRequiresSignInWithoutLosingTheProfile() throws {
    let fixture = try fixture()
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"]
    app.launchEnvironment["PINVOYAGER_TEST_RESPONSES"] = String(
      decoding: try JSONSerialization.data(withJSONObject: fixture.replies), as: UTF8.self)
    app.launch()
    app.buttons["View traveler"].firstMatch.tap()
    XCTAssertTrue(app.staticTexts["fixture-traveler"].waitForExistence(timeout: 5))
    app.buttons["More"].tap()
    app.buttons["Block traveler"].tap()
    XCTAssertTrue(app.buttons["Cancel"].waitForExistence(timeout: 5))
    app.buttons["Cancel"].tap()
    XCTAssertTrue(app.staticTexts["fixture-traveler"].exists)
  }

  private func fixture() throws -> FixtureLaunch {
    var fixture = try FixtureLaunch()
    let owner = UUID()
    var list = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_list")) as? [String: Any]
    )
    var voyages = try XCTUnwrap(list["voyages"] as? [[String: Any]])
    voyages[0]["owner_id"] = owner.uuidString
    list["voyages"] = voyages
    fixture.reply("/api/voyages", body: try JSONSerialization.data(withJSONObject: list))
    fixture.reply(
      "/api/profiles/\(owner)",
      body: try JSONSerialization.data(withJSONObject: [
        "user": ["id": owner.uuidString, "display_name": "fixture-traveler"],
        "voyages": voyages, "next_before": "next-profile-page",
      ]))
    return fixture
  }
}
