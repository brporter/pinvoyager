import UIKit
import XCTest

@MainActor
final class NavigationTests: UITestCase {
  func testNewOfflineDraftCanAdvanceDirectlyToPhotoImportWithoutAServerIdentifier() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture", "--offline-fixture"]
    app.launchFixture(tab: .voyages)
    XCTAssertTrue(app.navigationBars["My Voyages"].waitUntilExists(timeout: 5))
    app.navigationBars["My Voyages"].buttons["Create a voyage"].tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitUntilExists(timeout: 5))
    title.tap()
    title.typeText("Offline photo walk")
    let description = app.textViews["Description"]
    description.tap()
    description.typeText("Saved before choosing photos")
    let add = app.buttons["Add photos"]
    app.reveal(add)
    XCTAssertTrue(add.exists)
    guard add.exists else { return }
    add.tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitUntilExists(timeout: 5))
    app.buttons["Close"].tap()
    XCTAssertTrue(app.navigationBars["Offline photo walk"].waitUntilExists(timeout: 5))
    XCTAssertTrue(app.staticTexts["Saved on this iPhone"].exists)
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertEqual(app.staticTexts.matching(identifier: "Offline photo walk").count, 1)
  }

  func testGuestTabsAndExplicitSignIn() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"]
    app.launchFixture()
    XCTAssertTrue(app.buttons["Discover"].waitUntilExists(timeout: 10))
    app.buttons["Account"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitUntilExists(timeout: 5))
    XCTAssertFalse(app.buttons["Continue with Apple"].exists)
    app.buttons["My Voyages"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to see your voyages"].exists)
    app.buttons["Sync"].tap()
    XCTAssertTrue(app.staticTexts["No pending work"].exists)
  }

  func testLocalDraftCreationAndEditing() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture"]
    app.launchFixture(tab: .voyages)
    app.buttons["Create a voyage"].firstMatch.tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitUntilExists(timeout: 5))
    title.tap()
    title.typeText("Winter walk")
    let description = app.textViews["Description"]
    description.tap()
    description.typeText("Snow on the trail")
    app.buttons["Save draft and close"].tap()
    XCTAssertTrue(app.staticTexts["Winter walk"].waitUntilExists(timeout: 5))
    XCTAssertTrue(app.staticTexts["Draft"].exists)
  }

  func testUnfinishedDraftIsRetainedAndCanBeDiscarded() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture"]
    app.launchFixture(tab: .voyages)
    app.buttons["Create a voyage"].firstMatch.tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitUntilExists(timeout: 5))
    title.tap()
    title.typeText("Unfinished")
    app.buttons["Close"].tap()
    app.buttons["Keep draft"].tap()
    XCTAssertTrue(app.navigationBars["New voyage"].waitForNonExistence(timeout: 5))
    let continuation = app.buttons["Continue unfinished draft"]
    XCTAssertTrue(continuation.waitUntilExists(timeout: 5))
    continuation.tap()
    XCTAssertEqual(app.textFields["Voyage name"].value as? String, "Unfinished")
    app.buttons["Close"].tap()
    app.buttons["Discard draft"].tap()
    XCTAssertTrue(app.buttons["Continue unfinished draft"].waitForNonExistence(timeout: 5))
  }

  func testPublicationRemainsLocalAndCanBeCanceled() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing",
      "--signed-in-fixture",
      "--populated-fixture",
      "--offline-fixture",
    ]
    app.launchFixture(tab: .voyages)
    let voyage = app.staticTexts["Winter in Hokkaido"]
    XCTAssertTrue(voyage.waitUntilExists(timeout: 5))
    voyage.tap()
    app.swipeUp()
    let review = app.buttons["Review publication"]
    XCTAssertTrue(review.waitUntilExists(timeout: 5))
    review.tap()
    let consent = app.switches["I approve this exact photo list and location precision"]
    XCTAssertTrue(consent.waitUntilExists(timeout: 5))
    consent.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    let publish = app.buttons["Queue publication"]
    XCTAssertTrue(publish.isEnabled, app.debugDescription)
    publish.tap()
    XCTAssertTrue(app.staticTexts["Publication queued on this iPhone"].waitUntilExists(timeout: 5))
    app.buttons["Close"].tap()
    app.buttons["Sync"].tap()
    XCTAssertTrue(
      app.staticTexts["Queued on this iPhone — not published"].waitUntilExists(timeout: 5)
    )
    app.buttons["Cancel publication"].tap()
    XCTAssertTrue(app.staticTexts["Canceled"].waitUntilExists(timeout: 5))
  }

  func testSignOutLocksSavedOwnerContent() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing",
      "--signed-in-fixture",
      "--populated-fixture",
      "--offline-fixture",
    ]
    app.launchFixture(tab: .account)
    XCTAssertTrue(app.staticTexts["Saved on this iPhone"].waitUntilExists(timeout: 5))
    app.buttons["Sign out"].tap()
    app.buttons["Keep drafts on this device and sign out"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitUntilExists(timeout: 5))
    app.buttons["My Voyages"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to see your voyages"].exists)
    XCTAssertFalse(app.staticTexts["Winter in Hokkaido"].exists)
  }
}

extension NavigationTests {
  func testRetainedPhotoRemovalUndoAndCameraFallback() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing",
      "--signed-in-fixture",
      "--populated-fixture",
      "--offline-fixture",
    ]
    app.launchFixture(tab: .voyages)
    app.staticTexts["Winter in Hokkaido"].tap()
    let photo = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture.jpg"))
      .firstMatch
    app.reveal(photo)
    XCTAssertTrue(photo.waitUntilExists(timeout: 5))
    photo.swipeLeft()
    app.buttons["Remove"].tap()
    app.buttons["Remove from voyage"].tap()
    XCTAssertTrue(app.buttons["Undo removal"].waitUntilExists(timeout: 5))
    app.buttons["Undo removal"].tap()
    XCTAssertTrue(photo.waitUntilExists(timeout: 5))
    app.buttons["Add photos"].tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitUntilExists(timeout: 5))
    app.buttons["Take photo"].tap()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let denial = springboard.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Allow"))
      .firstMatch
    if denial.waitUntilExists(timeout: 2) { denial.tap() }
    XCTAssertTrue(app.staticTexts["Camera unavailable"].waitUntilExists(timeout: 10))
    app.buttons["Choose photos instead"].tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitUntilExists(timeout: 5))
    app.buttons["Close"].tap()
    XCTAssertTrue(app.buttons["Add photos"].waitUntilExists(timeout: 5))
  }
}

extension NavigationTests {
  func testAnExplicitSmallerCopyReplacesThePhotoAfterSaving() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing",
      "--signed-in-fixture",
      "--populated-fixture",
      "--offline-fixture",
      "--large-photo-fixture",
    ]
    app.launchFixture(tab: .voyages)
    app.staticTexts["Winter in Hokkaido"].tap()
    let photo = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture.jpg"))
      .firstMatch
    app.reveal(photo)
    XCTAssertTrue(photo.waitUntilExists(timeout: 5))
    photo.swipeLeft()
    app.buttons["Replace source"].tap()
    XCTAssertTrue(app.buttons["Create smaller JPEG copy"].waitUntilExists(timeout: 5))
    app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "JPEG longest edge")).firstMatch
      .tap()
    app.buttons["512 pixels"].tap()
    app.buttons["Create smaller JPEG copy"].tap()
    XCTAssertTrue(app.staticTexts["Smaller JPEG copy"].waitUntilExists(timeout: 10))
    XCTAssertFalse(app.staticTexts["Fixture.jpg"].exists)
  }
}

extension NavigationTests {
  func testCapturedPhotographsCanBeRetakenAndSavedThroughTheImportService() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing",
      "--signed-in-fixture",
      "--populated-fixture",
      "--offline-fixture",
    ]
    let image = UIGraphicsImageRenderer(size: CGSize(width: 80, height: 80)).jpegData(
      withCompressionQuality: 0.8
    ) { context in
      UIColor.systemOrange.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 80, height: 80))
    }
    app.launchEnvironment["PINVOYAGER_TEST_CAMERA"] = image.base64EncodedString()
    app.launchFixture(tab: .voyages)
    app.staticTexts["Winter in Hokkaido"].tap()
    app.swipeUp()
    app.buttons["Add photos"].tap()
    app.buttons["Take photo"].tap()
    XCTAssertTrue(app.switches["Add location"].waitUntilExists(timeout: 5))
    app.switches["Add location"].tap()
    app.switches["Flash"].tap()
    app.buttons["Switch camera"].tap()
    app.buttons["camera.capture"].tap()
    XCTAssertTrue(app.images["Captured photo"].waitUntilExists(timeout: 5))
    XCTAssertTrue(app.staticTexts["No location"].exists)
    app.buttons["Retake"].tap()
    app.buttons["camera.capture"].tap()
    app.buttons["Use photo"].tap()
    XCTAssertTrue(
      app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Capture-")).firstMatch
        .waitUntilExists(timeout: 5)
    )
    app.buttons["Close"].tap()
    XCTAssertTrue(app.staticTexts["2 photos · 2 without location"].waitUntilExists(timeout: 5))
  }
}

extension NavigationTests {
  func testPrecisionReviewUsesAnOfflineExampleAndCancelKeepsThePreviousChoice() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture", "--offline-fixture"]
    app.launchFixture(tab: .voyages)
    app.buttons["Create a voyage"].firstMatch.tap()
    app.buttons["Location precision"].tap()
    XCTAssertTrue(
      app.staticTexts["Map unavailable offline — schematic example"].waitUntilExists(timeout: 5)
    )
    app.buttons["Approximately 50 km"].tap()
    app.buttons["Cancel"].tap()
    XCTAssertTrue(app.navigationBars["Location precision"].waitForNonExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Location precision"].waitUntilExists(timeout: 5))
    app.buttons["Location precision"].tap()
    app.buttons["Exact"].tap()
    XCTAssertTrue(
      app.staticTexts["Exact recorded locations will be public"].waitUntilExists(timeout: 5)
    )
    app.buttons["Done"].tap()
    XCTAssertTrue(
      app.staticTexts["Exact recorded locations will be public"].waitUntilExists(timeout: 5)
    )
  }
}
