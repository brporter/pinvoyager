import UIKit
import XCTest

@MainActor
final class NavigationTests: XCTestCase {
  func testNewOfflineDraftCanAdvanceDirectlyToPhotoImportWithoutAServerIdentifier() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture", "--offline-fixture"]
    app.launch()
    app.buttons["Create a voyage"].tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitForExistence(timeout: 5))
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
    XCTAssertTrue(app.buttons["Choose photos"].waitForExistence(timeout: 5))
    app.buttons["Close"].tap()
    XCTAssertTrue(app.navigationBars["Offline photo walk"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Saved on this iPhone"].exists)
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertEqual(app.staticTexts.matching(identifier: "Offline photo walk").count, 1)
  }

  func testGuestTabsAndExplicitSignIn() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"]
    app.launch()
    XCTAssertTrue(app.buttons["Discover"].waitForExistence(timeout: 10))
    app.buttons["Account"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Continue with Apple"].exists)
    app.buttons["My Voyages"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to see your voyages"].exists)
    app.buttons["Sync"].tap()
    XCTAssertTrue(app.staticTexts["No pending work"].exists)
  }

  func testLocalDraftCreationAndEditing() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture"]
    app.launch()
    app.buttons["My Voyages"].tap()
    app.buttons["Create a voyage"].firstMatch.tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitForExistence(timeout: 5))
    title.tap()
    title.typeText("Winter walk")
    let description = app.textViews["Description"]
    description.tap()
    description.typeText("Snow on the trail")
    app.buttons["Save draft and close"].tap()
    XCTAssertTrue(app.staticTexts["Winter walk"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Draft"].exists)
  }

  func testUnfinishedDraftIsRetainedAndCanBeDiscarded() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture"]
    app.launch()
    app.buttons["My Voyages"].tap()
    app.buttons["Create a voyage"].firstMatch.tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitForExistence(timeout: 5))
    title.tap()
    title.typeText("Unfinished")
    app.buttons["Close"].tap()
    app.buttons["Keep draft"].tap()
    let continuation = app.buttons["Continue unfinished draft"]
    XCTAssertTrue(continuation.waitForExistence(timeout: 5))
    continuation.tap()
    XCTAssertEqual(app.textFields["Voyage name"].value as? String, "Unfinished")
    app.buttons["Close"].tap()
    app.buttons["Discard draft"].tap()
    XCTAssertTrue(app.buttons["Continue unfinished draft"].waitForNonExistence(timeout: 5))
  }

  func testPublicationRemainsLocalAndCanBeCanceled() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
    ]
    app.launch()
    app.buttons["My Voyages"].tap()
    let voyage = app.staticTexts["Winter in Hokkaido"]
    XCTAssertTrue(voyage.waitForExistence(timeout: 5))
    voyage.tap()
    app.swipeUp()
    let review = app.buttons["Review publication"]
    XCTAssertTrue(review.waitForExistence(timeout: 5))
    review.tap()
    let consent = app.switches["I approve this exact photo list and location precision"]
    XCTAssertTrue(consent.waitForExistence(timeout: 5))
    consent.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    let publish = app.buttons["Queue publication"]
    XCTAssertTrue(publish.isEnabled, app.debugDescription)
    publish.tap()
    XCTAssertTrue(app.staticTexts["Publication queued on this iPhone"].waitForExistence(timeout: 5))
    app.buttons["Close"].tap()
    app.buttons["Sync"].tap()
    XCTAssertTrue(
      app.staticTexts["Queued on this iPhone — not published"].waitForExistence(timeout: 5))
    app.buttons["Cancel publication"].tap()
    XCTAssertTrue(app.staticTexts["Canceled"].waitForExistence(timeout: 5))
  }

  func testSignOutLocksSavedOwnerContent() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
    ]
    app.launch()
    app.buttons["Account"].tap()
    XCTAssertTrue(app.staticTexts["Saved on this iPhone"].waitForExistence(timeout: 5))
    app.buttons["Sign out"].tap()
    app.buttons["Keep drafts on this device and sign out"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitForExistence(timeout: 5))
    app.buttons["My Voyages"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to see your voyages"].exists)
    XCTAssertFalse(app.staticTexts["Winter in Hokkaido"].exists)
  }
}

extension NavigationTests {
  func testRetainedPhotoRemovalUndoAndCameraFallback() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
    ]
    app.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts["Winter in Hokkaido"].tap()
    let photo = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture.jpg"))
      .firstMatch
    app.reveal(photo)
    XCTAssertTrue(photo.waitForExistence(timeout: 5))
    photo.swipeLeft()
    app.buttons["Remove"].tap()
    app.buttons["Remove from voyage"].tap()
    XCTAssertTrue(app.buttons["Undo removal"].waitForExistence(timeout: 5))
    app.buttons["Undo removal"].tap()
    XCTAssertTrue(photo.waitForExistence(timeout: 5))
    app.buttons["Add photos"].tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitForExistence(timeout: 5))
    app.buttons["Take photo"].tap()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let denial = springboard.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Allow"))
      .firstMatch
    if denial.waitForExistence(timeout: 2) { denial.tap() }
    XCTAssertTrue(app.staticTexts["Camera unavailable"].waitForExistence(timeout: 10))
    app.buttons["Choose photos instead"].tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitForExistence(timeout: 5))
    app.buttons["Close"].tap()
    XCTAssertTrue(app.buttons["Add photos"].waitForExistence(timeout: 5))
  }
}

extension NavigationTests {
  func testAnExplicitSmallerCopyReplacesThePhotoAfterSaving() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
      "--large-photo-fixture",
    ]
    app.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts["Winter in Hokkaido"].tap()
    let photo = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture.jpg"))
      .firstMatch
    app.reveal(photo)
    XCTAssertTrue(photo.waitForExistence(timeout: 5))
    photo.swipeLeft()
    app.buttons["Replace source"].tap()
    XCTAssertTrue(app.buttons["Create smaller JPEG copy"].waitForExistence(timeout: 5))
    app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "JPEG longest edge")).firstMatch
      .tap()
    app.buttons["512 pixels"].tap()
    app.buttons["Create smaller JPEG copy"].tap()
    XCTAssertTrue(app.staticTexts["Smaller JPEG copy"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.staticTexts["Fixture.jpg"].exists)
  }
}

extension NavigationTests {
  func testCapturedPhotographsCanBeRetakenAndSavedThroughTheImportService() {
    let app = XCUIApplication()
    app.launchArguments = [
      "--ui-testing", "--signed-in-fixture", "--populated-fixture", "--offline-fixture",
    ]
    let image = UIGraphicsImageRenderer(size: CGSize(width: 80, height: 80)).jpegData(
      withCompressionQuality: 0.8
    ) { context in
      UIColor.systemOrange.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 80, height: 80))
    }
    app.launchEnvironment["PINVOYAGER_TEST_CAMERA"] = image.base64EncodedString()
    app.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts["Winter in Hokkaido"].tap()
    app.swipeUp()
    app.buttons["Add photos"].tap()
    app.buttons["Take photo"].tap()
    XCTAssertTrue(app.switches["Add location"].waitForExistence(timeout: 5))
    app.switches["Add location"].tap()
    app.switches["Flash"].tap()
    app.buttons["Switch camera"].tap()
    app.buttons["camera.capture"].tap()
    XCTAssertTrue(app.images["Captured photo"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["No location"].exists)
    app.buttons["Retake"].tap()
    app.buttons["camera.capture"].tap()
    app.buttons["Use photo"].tap()
    XCTAssertTrue(
      app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Capture-")).firstMatch
        .waitForExistence(timeout: 5))
    app.buttons["Close"].tap()
    XCTAssertTrue(app.staticTexts["2 photos · 2 without location"].waitForExistence(timeout: 5))
  }
}

extension NavigationTests {
  func testPrecisionReviewUsesAnOfflineExampleAndCancelKeepsThePreviousChoice() {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing", "--signed-in-fixture", "--offline-fixture"]
    app.launch()
    app.buttons["My Voyages"].tap()
    app.buttons["Create a voyage"].firstMatch.tap()
    app.buttons["Location precision"].tap()
    XCTAssertTrue(
      app.staticTexts["Map unavailable offline — schematic example"].waitForExistence(timeout: 5))
    app.buttons["Approximately 50 km"].tap()
    app.buttons["Cancel"].tap()
    XCTAssertTrue(app.buttons["Location precision"].waitForExistence(timeout: 5))
    app.buttons["Location precision"].tap()
    app.buttons["Exact"].tap()
    XCTAssertTrue(
      app.staticTexts["Exact recorded locations will be public"].waitForExistence(timeout: 5))
    app.buttons["Done"].tap()
    XCTAssertTrue(
      app.staticTexts["Exact recorded locations will be public"].waitForExistence(timeout: 5))
  }
}
