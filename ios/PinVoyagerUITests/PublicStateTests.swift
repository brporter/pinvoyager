import PinVoyagerData
import XCTest

@MainActor final class PublicStateTests: XCTestCase {
  func testCorruptImageShowsAnErrorAndProcessingAndFailedPhotosRemainDistinct() throws {
    var fixture = try FixtureLaunch()
    var detail = try Self.detail()
    detail["owner_id"] = UUID().uuidString
    var photos = try XCTUnwrap(detail["photos"] as? [[String: Any]])
    photos[0]["variant_status"] = "pending"
    photos[1]["variant_status"] = "failed"
    detail["photos"] = photos
    fixture.reply("/api/voyages", body: try FixtureLaunch.fixture("voyage_list"))
    fixture.reply(
      "/api/voyages/\(detail["id"]!)", body: try JSONSerialization.data(withJSONObject: detail))
    fixture.reply(
      try XCTUnwrap(photos[2]["display_url"] as? String), body: Data("not an image".utf8))
    let app = try fixture.launch()
    app.staticTexts["Three weeks in Hokkaido"].firstMatch.tap()
    app.buttons["Photos"].tap()
    XCTAssertTrue(app.staticTexts["Processing"].exists)
    XCTAssertTrue(app.staticTexts["Could not process"].exists)
    let photo = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Photo 4"))
      .firstMatch
    app.reveal(photo)
    photo.tap()
    XCTAssertTrue(
      app.staticTexts["This photograph could not be decoded."].waitForExistence(timeout: 5))
    app.buttons["Close"].tap()
    app.buttons["More"].tap()
    app.buttons["Report voyage"].tap()
    XCTAssertTrue(app.buttons["Submit report"].waitForExistence(timeout: 5))
    app.buttons["Cancel"].tap()
  }

  func testEmptyOwnProfileOffersMyVoyagesAndAnEmptyVoyageShowsNoPhotos() throws {
    var fixture = try FixtureLaunch()
    var detail = try Self.detail()
    detail["owner_id"] = fixture.workspace.account.id.uuidString
    detail["photos"] = []
    for key in ["photo_count", "placed_count", "unplaced_count", "place_count"] { detail[key] = 0 }
    fixture.reply("/api/voyages", body: try FixtureLaunch.fixture("voyage_list"))
    fixture.reply(
      "/api/voyages/\(detail["id"]!)", body: try JSONSerialization.data(withJSONObject: detail))
    fixture.reply(
      "/api/profiles/\(fixture.workspace.account.id)",
      body: try JSONSerialization.data(withJSONObject: [
        "user": ["id": fixture.workspace.account.id.uuidString, "display_name": NSNull()],
        "voyages": [], "next_before": NSNull(),
      ]))
    let app = try fixture.launch()
    app.staticTexts["Three weeks in Hokkaido"].firstMatch.tap()
    XCTAssertTrue(app.staticTexts["No photos"].waitForExistence(timeout: 5))
    app.buttons["View traveler"].tap()
    XCTAssertTrue(app.staticTexts["No public voyages"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Traveler"].exists)
    try XCTUnwrap(
      app.buttons.matching(identifier: "My Voyages").allElementsBoundByIndex.first { $0.isHittable }
    ).tap()
    XCTAssertTrue(app.staticTexts["No saved voyages"].waitForExistence(timeout: 5))
  }

  func testUnavailableVoyageOffersRetryWithoutDisplayingItsOldMetadata() throws {
    var fixture = try FixtureLaunch()
    let detail = try Self.detail()
    fixture.reply("/api/voyages", body: try FixtureLaunch.fixture("voyage_list"))
    fixture.reply("/api/voyages/\(detail["id"]!)", body: Data("{}".utf8), status: 503)
    let app = try fixture.launch()
    app.staticTexts["Three weeks in Hokkaido"].firstMatch.tap()
    let retry = app.buttons["Try again"]
    XCTAssertTrue(retry.waitForExistence(timeout: 5))
    retry.tap()
    XCTAssertTrue(retry.waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["View traveler"].exists)
  }

  private static func detail() throws -> [String: Any] {
    try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_detail"))
        as? [String: Any])
  }
}
