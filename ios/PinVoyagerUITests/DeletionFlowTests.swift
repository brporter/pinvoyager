import PinVoyagerData
import XCTest

@MainActor
final class DeletionFlowTests: UITestCase {
  func testPreparedDeletionRequiresConfirmationAndItsStatusSurvivesSignOut() throws {
    var fixture = try FixtureLaunch()
    let record = try fixture.deletion(status: .prepared)
    fixture.deletions = [record]
    fixture.reply(
      "/api/me/deletion-requests/\(record.id)/confirm",
      body: try JSONSerialization.data(withJSONObject: [
        "id": record.id.uuidString,
        "status": "complete",
      ]),
      status: 202
    )
    fixture.reply(
      "/api/account-deletions/\(record.id)",
      body: Data("{\"status\":\"complete\"}".utf8)
    )
    fixture.reply("/auth/native/logout", body: Data(), status: 204)
    let app = try fixture.launch(tab: .account)
    app.buttons["Delete account"].tap()
    XCTAssertTrue(
      app.staticTexts["Your account is ready for deletion. It has not been deleted yet."]
        .waitUntilExists(timeout: 5)
    )
    app.buttons["Confirm account deletion"].tap()
    app.buttons["Delete my account"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitUntilExists(timeout: 5))
    app.buttons["Account deletion status"].tap()
    XCTAssertTrue(app.staticTexts["Account deletion is complete."].waitUntilExists(timeout: 5))
    app.buttons["Check deletion status"].tap()
    app.buttons["Remove this status record"].tap()
    XCTAssertTrue(app.staticTexts["Account deletion is complete."].waitForNonExistence(timeout: 5))
  }

  func testUnknownPreparationCanBeRetriedOrCheckedWithoutAnotherProviderLogin() throws {
    for action in ["Retry preparation", "Check preparation status"] {
      var fixture = try FixtureLaunch()
      let record = try fixture.deletion(status: .preparing)
      fixture.deletions = [record]
      fixture.reply(
        "/api/me/deletion-requests",
        body: try JSONSerialization.data(withJSONObject: [
          "id": record.id.uuidString,
          "status": "prepared",
          "expires_at": ISO8601DateFormatter().string(from: record.expiresAt),
        ]),
        status: 201
      )
      fixture.reply(
        "/api/account-deletions/\(record.id)",
        body: Data("{\"status\":\"prepared\"}".utf8)
      )
      let app = try fixture.launch(tab: .account)
      app.buttons["Delete account"].tap()
      XCTAssertTrue(app.buttons[action].waitUntilExists(timeout: 5))
      app.buttons[action].tap()
      XCTAssertTrue(app.buttons["Confirm account deletion"].waitUntilExists(timeout: 5))
    }
  }
}
