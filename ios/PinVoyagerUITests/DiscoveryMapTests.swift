import XCTest

@MainActor final class DiscoveryMapTests: XCTestCase {
  func testSwitchingBetweenRecentAndMapPreservesTheRecentScrollPosition() throws {
    var fixture = try FixtureLaunch()
    let page = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_list"))
        as? [String: Any])
    let template = try XCTUnwrap((page["voyages"] as? [[String: Any]])?.first)
    let voyages = (0..<12).map { index -> [String: Any] in
      var voyage = template
      voyage["id"] = UUID().uuidString
      voyage["title"] = "Discovery voyage \(index)"
      return voyage
    }
    fixture.reply(
      "/api/voyages",
      body: try JSONSerialization.data(withJSONObject: [
        "voyages": voyages, "next_before": NSNull(),
      ]))
    let app = try fixture.launch()
    let target = app.staticTexts["Discovery voyage 6"]
    app.reveal(target, maximumSwipes: 20)
    XCTAssertTrue(target.isHittable)
    app.buttons["Map"].tap()
    app.buttons["Recent"].tap()
    XCTAssertTrue(
      target.isHittable, "Discover must retain the position reached before switching modes.")
  }

  func testDiscoveryClustersZoomThenOfferVoyagesAtTheMaximumZoom() throws {
    var fixture = try FixtureLaunch()
    var map = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("discover_map"))
        as? [String: Any])
    var clusters = try XCTUnwrap(map["clusters"] as? [[String: Any]])
    clusters[0]["coordinate"] = ["latitude": 25.0, "longitude": 0.0, "precision_m": 5000]
    map["clusters"] = clusters
    map["truncated"] = true
    fixture.reply("/api/map", body: try JSONSerialization.data(withJSONObject: map))
    let detail = try FixtureLaunch.fixture("voyage_detail")
    let voyage = try XCTUnwrap(try JSONSerialization.jsonObject(with: detail) as? [String: Any])
    fixture.reply("/api/voyages/" + (try XCTUnwrap(voyage["id"] as? String)), body: detail)
    let app = try fixture.launch()
    app.buttons["Map"].tap()
    XCTAssertTrue(app.staticTexts["Zoom in to see more photographs"].waitForExistence(timeout: 10))
    let pin = app.buttons["2 photographs in 1 voyages"]
    for _ in 0..<13 {
      if app.navigationBars["Voyages at this location"].exists { break }
      XCTAssertTrue(pin.waitForExistence(timeout: 5))
      pin.tap()
    }
    XCTAssertTrue(app.navigationBars["Voyages at this location"].waitForExistence(timeout: 5))
    app.buttons.matching(
      NSPredicate(format: "identifier BEGINSWITH %@", "discovery.cluster-voyage.")
    ).firstMatch.tap()
    XCTAssertTrue(app.buttons["View traveler"].waitForExistence(timeout: 5))
  }
}
