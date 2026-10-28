import XCTest

@MainActor final class ReportingFlowTests: XCTestCase {
  func testReportReceiptAndBlockingHidePreviouslyLoadedTravelerContent() throws {
    var fixture = try FixtureLaunch()
    let peer = UUID()
    let owner = fixture.workspace.account.id
    var page = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_list")) as? [String: Any]
    )
    var summaries = try XCTUnwrap(page["voyages"] as? [[String: Any]])
    for index in summaries.indices { summaries[index]["owner_id"] = peer.uuidString }
    page["voyages"] = summaries
    fixture.reply("/api/voyages", body: try JSONSerialization.data(withJSONObject: page))
    var voyage = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_detail"))
        as? [String: Any])
    voyage["owner_id"] = peer.uuidString
    fixture.reply(
      "/api/voyages/" + (try XCTUnwrap(voyage["id"] as? String)),
      body: try JSONSerialization.data(withJSONObject: voyage))
    fixture.reply(
      "/api/client-capabilities", body: try FixtureLaunch.fixture("native_capabilities"))
    let receipt = UUID()
    fixture.reply(
      "/api/reports",
      body: try JSONSerialization.data(withJSONObject: ["id": receipt.uuidString, "status": "open"]
      ), status: 201)
    fixture.reply(
      "/api/sync/operations/*",
      body: try JSONSerialization.data(withJSONObject: [
        "status": "applied", "target_id": receipt.uuidString, "revision": "0",
      ]))
    let app = try fixture.launch()
    XCTAssertNotEqual(owner, peer)
    app.staticTexts["Three weeks in Hokkaido"].firstMatch.tap()
    app.buttons["More"].tap()
    app.buttons["Report voyage"].tap()
    app.textViews["Report details"].tap()
    app.textViews["Report details"].typeText("Please review this content.")
    app.buttons["Submit report"].tap()
    XCTAssertTrue(app.staticTexts["Report received"].waitForExistence(timeout: 5))
    app.buttons["Done"].tap()
    app.buttons["More"].tap()
    app.buttons["Block traveler"].tap()
    XCTAssertTrue(app.staticTexts["Traveler blocked"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(app.staticTexts["Three weeks in Hokkaido"].waitForNonExistence(timeout: 5))
    app.buttons["Map"].tap()
    XCTAssertTrue(
      app.staticTexts["Map waiting for blocked-account changes"].waitForExistence(timeout: 5))
  }
}
