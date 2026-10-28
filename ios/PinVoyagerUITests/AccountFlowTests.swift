import PinVoyagerDomain
import XCTest

@MainActor final class AccountFlowTests: XCTestCase {
  func testAccountNameMethodsBlockingExportAndHelp() throws {
    var fixture = try FixtureLaunch()
    fixture.workspace.blockedAccounts = [UUID()]
    fixture.workspace.voyages = [
      LocalVoyage(
        ownerId: fixture.workspace.account.id,
        fields: VoyageFields(title: "Winter in Hokkaido", description: "Test voyage"))
    ]
    fixture.reply(
      "/api/me", body: Data("{\"message\":\"That name is unavailable.\"}".utf8), status: 409)
    let app = try fixture.launch()
    app.buttons["Account"].tap()
    app.buttons["Change display name"].tap()
    let field = app.textFields["Display name"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("-new")
    app.buttons["Save"].tap()
    XCTAssertTrue(app.staticTexts["That name is unavailable."].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Sign-in methods"].tap()
    XCTAssertTrue(app.staticTexts["Linked"].waitForExistence(timeout: 5))
    app.buttons["Link"].firstMatch.tap()
    XCTAssertTrue(app.buttons["Continue with Apple"].waitForExistence(timeout: 5))
    app.switches["Use another account"].tap()
    app.buttons["Continue with Apple"].tap()
    XCTAssertTrue(app.staticTexts["This content is unavailable."].waitForExistence(timeout: 5))
    app.buttons["Cancel"].tap()
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Blocked accounts"].tap()
    XCTAssertTrue(app.buttons["Unblock"].waitForExistence(timeout: 5))
    app.buttons["Unblock"].tap()
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Delete account"].tap()
    XCTAssertTrue(
      app.staticTexts["Verify a linked sign-in method before confirming deletion."]
        .waitForExistence(timeout: 5))
    app.buttons["Verify with Google"].tap()
    XCTAssertTrue(app.staticTexts["This content is unavailable."].waitForExistence(timeout: 5))
    app.buttons["Export local work"].tap()
    XCTAssertTrue(app.staticTexts["Winter in Hokkaido"].waitForExistence(timeout: 5))
    app.staticTexts["Winter in Hokkaido"].tap()
    app.buttons["Prepare export of local work"].tap()
    app.reveal(app.buttons["Export local work"])
    XCTAssertTrue(app.buttons["Export local work"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Privacy, terms and help"].tap()
    XCTAssertTrue(app.buttons["Share diagnostic references"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.links["Contact support"].exists)
  }
}

extension AccountFlowTests {
  func testBundledLegalPoliciesOpenWithoutConnection() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture", "--offline-fixture"]
    app.launch()
    app.buttons["Account"].tap()
    app.buttons["Privacy, terms and help"].tap()
    for (button, title) in [("Privacy", "Privacy Policy"), ("Terms", "Terms of Service")] {
      app.buttons[button].tap()
      XCTAssertTrue(app.navigationBars[title].waitForExistence(timeout: 5))
      XCTAssertTrue(app.staticTexts["Bundled revision: 2026-10-18"].exists)
      let back = app.navigationBars[title].buttons.element(boundBy: 0)
      XCTAssertTrue(back.isHittable)
      back.tap()
      XCTAssertTrue(app.navigationBars["Help"].waitForExistence(timeout: 5))
    }
  }
}

extension AccountFlowTests {
  func testSignedOutNetworkPreferencesSurviveRelaunch() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"]
    app.launchEnvironment["PINVOYAGER_TEST_PREFERENCES"] =
      "com.pinvoyager.ui-test.preferences.\(UUID())"
    app.launch()
    app.buttons["Account"].tap()
    app.buttons["Sync and storage"].tap()
    let wifi = app.switches["Upload photos on Wi-Fi only"]
    XCTAssertEqual(wifi.value as? String, "1")
    wifi.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    let constrained = app.switches["Use Low Data Mode connections"]
    constrained.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    XCTAssertEqual(wifi.value as? String, "0")
    XCTAssertEqual(constrained.value as? String, "1")
    app.navigationBars["Sync and storage"].buttons.element(boundBy: 0).tap()
    app.terminate()
    app.launch()
    app.buttons["Account"].tap()
    app.buttons["Sync and storage"].tap()
    XCTAssertEqual(wifi.value as? String, "0")
    XCTAssertEqual(constrained.value as? String, "1")
    XCTAssertFalse(app.switches["Notify when publication completes"].isEnabled)
  }
}

extension AccountFlowTests {
  func testStorageControlsRequireCleanupConsentAndOfferVoyageExportsAndDownloads() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
    ]
    app.launch()
    app.buttons["Account"].tap()
    app.buttons["Sync and storage"].tap()
    app.reveal(app.switches["Keep uploaded originals on this device"])
    app.switches["Keep uploaded originals on this device"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)
    ).tap()
    XCTAssertTrue(app.buttons["Remove eligible originals"].waitForExistence(timeout: 5))
    app.buttons["Cancel"].tap()
    XCTAssertEqual(app.switches["Keep uploaded originals on this device"].value as? String, "1")
    app.switches["Keep uploaded originals on this device"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)
    ).tap()
    app.buttons["Remove eligible originals"].tap()
    XCTAssertEqual(app.switches["Keep uploaded originals on this device"].value as? String, "0")
    app.reveal(app.switches["Upload photos on Wi-Fi only"], swipeUp: false)
    app.switches["Upload photos on Wi-Fi only"].tap()
    app.reveal(app.switches["Keep uploaded originals on this device"])
    XCTAssertEqual(app.switches["Keep uploaded originals on this device"].value as? String, "0")
    app.reveal(app.buttons["Manage downloads"])
    app.buttons["Manage downloads"].tap()
    XCTAssertTrue(app.navigationBars["Manage downloads"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.reveal(app.buttons["Clear temporary cache"])
    app.buttons["Clear temporary cache"].tap()
    app.reveal(app.buttons["Export local draft"])
    app.buttons["Export local draft"].tap()
    app.staticTexts["Winter in Hokkaido"].tap()
    XCTAssertTrue(
      app.staticTexts["Exact photo locations may be included."].waitForExistence(timeout: 5))
    app.buttons["Prepare export of local work"].tap()
    XCTAssertTrue(app.buttons["Save export to Files"].waitForExistence(timeout: 5))
    app.buttons["Save export to Files"].tap()
    let cancelExport = app.descendants(matching: .any).matching(
      NSPredicate(format: "label == %@", "Cancel")
    ).firstMatch
    XCTAssertTrue(cancelExport.waitForExistence(timeout: 5), app.debugDescription)
    cancelExport.tap()
    XCTAssertTrue(app.buttons["Save export to Files"].waitForExistence(timeout: 5))
  }
}

extension AccountFlowTests {
  func testSignOutWithScheduledPublicationRequiresAnExplicitLeaveScheduledChoice() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(
        title: "Scheduled before sign out", description: "This may publish independently"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Scheduled photo")
    photo.serverId = UUID()
    photo.transfer = .accepted
    photo.variant = .ready
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    var intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: Date())
    intent.serverId = UUID()
    intent.status = .processing
    fixture.workspace.publicationIntents = [intent]
    fixture.reply("/auth/native/logout", body: Data(), status: 204)
    let app = try fixture.launch()
    app.buttons["Account"].tap()
    app.buttons["Sign out"].tap()
    let leave = app.buttons["Leave publication scheduled and sign out"]
    XCTAssertTrue(leave.waitForExistence(timeout: 5))
    guard leave.exists else { return }
    leave.tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitForExistence(timeout: 5))
  }
}

extension AccountFlowTests {
  func testPublicationWinningCancellationRequiresANewPrivacyOrSignOutChoice() throws {
    for leavePublic in [false, true] {
      var fixture = try FixtureLaunch()
      var capabilities = try XCTUnwrap(
        try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("native_capabilities"))
          as? [String: Any])
      capabilities["native_protocol_versions"] = [1]
      fixture.reply(
        "/api/client-capabilities", body: try JSONSerialization.data(withJSONObject: capabilities))
      var voyage = LocalVoyage(
        ownerId: fixture.workspace.account.id,
        fields: VoyageFields(
          title: "Publication won the race", description: "Do not claim it stayed private"))
      voyage.serverId = UUID()
      voyage.revision = "7"
      var photo = LocalPhoto(voyageId: voyage.id, filename: "Reviewed photograph")
      photo.serverId = UUID()
      photo.transfer = .accepted
      photo.variant = .ready
      var intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: Date())
      intent.serverId = UUID()
      intent.status = .processing
      var publish = OutboxOperation(
        accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
        kind: .publish(intentId: intent.id))
      publish.state = .applied
      fixture.workspace.voyages = [voyage]
      fixture.workspace.photos = [photo]
      fixture.workspace.publicationIntents = [intent]
      fixture.workspace.operations = [publish]
      fixture.reply(
        "/api/voyages/\(voyage.serverId!)/publication-intents/\(intent.serverId!)",
        body: try JSONSerialization.data(withJSONObject: [
          "id": intent.serverId!.uuidString, "status": "published", "revision": "7",
          "published": true,
        ]))
      fixture.reply(
        "/api/sync/operations/*",
        body: try JSONSerialization.data(withJSONObject: [
          "status": "applied", "publication_intent_id": intent.serverId!.uuidString,
          "publication_status": "published", "voyage_id": voyage.serverId!.uuidString,
          "revision": "7",
        ]))
      fixture.reply("/api/voyages/\(voyage.serverId!)", body: Data("{\"published\":true}".utf8))
      fixture.reply("/auth/native/logout", body: Data(), status: 204)
      let app = try fixture.launch()
      app.buttons["Account"].tap()
      app.buttons["Sign out"].tap()
      app.buttons["Cancel publication and sign out"].tap()
      let unpublish = app.buttons["Unpublish now and sign out"]
      XCTAssertTrue(unpublish.waitForExistence(timeout: 10), app.debugDescription)
      if leavePublic {
        app.buttons["Leave voyage public and sign out"].tap()
        XCTAssertTrue(app.staticTexts["Sign in to your account"].waitForExistence(timeout: 5))
      } else {
        app.buttons["Keep account open"].tap()
        XCTAssertTrue(unpublish.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Sign out"].exists)
      }
      app.terminate()
    }
  }
}
