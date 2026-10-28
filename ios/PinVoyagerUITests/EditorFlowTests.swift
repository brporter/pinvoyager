import PinVoyagerDomain
import XCTest

@MainActor final class EditorFlowTests: XCTestCase {
  func testExistingWorkingCopyCanBeDiscardedWithoutChangingTheCommittedVoyage() throws {
    var fixture = try FixtureLaunch()
    let fields = VoyageFields(title: "Committed voyage", description: nil)
    let voyage = LocalVoyage(ownerId: fixture.workspace.account.id, fields: fields)
    fixture.workspace.voyages = [voyage]
    fixture.workspace.workingCopies = [
      VoyageWorkingCopy(
        voyage: voyage,
        fields: VoyageFields(title: "Recovered working copy", description: "Uncommitted edits"))
    ]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts["Committed voyage"].tap()
    app.buttons["Edit details"].tap()
    let title = app.textFields["Voyage name"]
    XCTAssertTrue(title.waitForExistence(timeout: 5))
    XCTAssertEqual(title.value as? String, "Recovered working copy")
    app.buttons["Cancel"].tap()
    if app.buttons["Continue editing"].exists {
      app.buttons["Continue editing"].tap()
    } else {
      app.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.85)).tap()
    }
    XCTAssertEqual(title.value as? String, "Recovered working copy")
    app.buttons["Cancel"].tap()
    app.buttons["Discard edits"].tap()
    XCTAssertTrue(app.staticTexts["Committed voyage"].waitForExistence(timeout: 5))
    app.buttons["Edit details"].tap()
    XCTAssertEqual(title.value as? String, "Committed voyage")
    app.buttons["Cancel"].tap()
    XCTAssertTrue(app.buttons["Edit details"].waitForExistence(timeout: 5))
  }

  func testInvalidRecoveredDraftShowsLimitsAndKeepsItsFieldsAfterFailedSaving() throws {
    continueAfterFailure = false
    var fixture = try FixtureLaunch()
    fixture.workspace.scratch = VoyageFields(
      title: String(repeating: "x", count: 201),
      description: String(repeating: "y", count: 8_001))
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.buttons["Continue unfinished draft"].tap()
    XCTAssertTrue(app.staticTexts["201 / 200 characters"].waitForExistence(timeout: 5))
    XCTAssertLessThanOrEqual(
      app.textViews["Description"].frame.height, app.frame.height / 2,
      "Long descriptions must scroll inside the editor without expanding the form across screens.")
    let descriptionCount = app.staticTexts.matching(
      NSPredicate(format: "label ENDSWITH %@", "/ 8,000 characters")
    ).firstMatch
    app.reveal(descriptionCount)
    XCTAssertTrue(descriptionCount.exists, app.debugDescription)
    app.reveal(app.buttons["Save draft and close"])
    app.buttons["Save draft and close"].tap()
    XCTAssertTrue(app.navigationBars["New voyage"].exists)
    app.reveal(app.staticTexts["Use at most 200 characters for the voyage name."])
    XCTAssertTrue(
      app.staticTexts["Use at most 200 characters for the voyage name."].exists,
      app.debugDescription)
    app.reveal(app.textFields["Voyage name"], swipeUp: false)
    XCTAssertEqual((app.textFields["Voyage name"].value as? String)?.count, 201)
    app.buttons["Close"].tap()
    app.buttons["Discard draft"].tap()
    XCTAssertTrue(app.buttons["Continue unfinished draft"].waitForNonExistence(timeout: 5))
  }

  func testBackgroundingAnEditorKeepsTheSeparateWorkingCopy() throws {
    var fixture = try FixtureLaunch()
    let voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Saved before editing", description: "Keep this description"))
    fixture.workspace.voyages = [voyage]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    app.buttons["Edit details"].tap()
    let title = app.textFields["Voyage name"]
    title.tap()
    title.typeText(" locally")
    XCUIDevice.shared.press(.home)
    app.activate()
    XCTAssertTrue(title.waitForExistence(timeout: 5))
    XCTAssertEqual(title.value as? String, "Saved before editing locally")
    app.buttons["Save changes"].tap()
    XCTAssertTrue(app.staticTexts["Saved before editing locally"].waitForExistence(timeout: 5))
  }
}

extension EditorFlowTests {
  func testArmedPublicationAllowsASeparateWorkingCopyWithoutChangingTheReviewedVoyage() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Reviewed version", description: "The approved description"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Reviewed photo")
    photo.serverId = UUID()
    photo.variant = .ready
    photo.transfer = .accepted
    var intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: Date())
    intent.serverId = UUID()
    intent.status = .processing
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    fixture.workspace.publicationIntents = [intent]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    let edit = app.buttons["Edit details"]
    XCTAssertTrue(edit.isEnabled)
    guard edit.isEnabled else { return }
    edit.tap()
    let title = app.textFields["Voyage name"]
    title.tap()
    title.typeText(" locally\n")
    let save = app.buttons["Save working copy on this iPhone"]
    app.reveal(save)
    save.tap()
    XCTAssertTrue(app.staticTexts["Reviewed version"].waitForExistence(timeout: 5))
    app.buttons["Sync"].tap()
    XCTAssertTrue(app.staticTexts["Processing"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.staticTexts["Save voyage details"].exists)
    app.buttons["My Voyages"].tap()
    app.buttons["Edit details"].tap()
    XCTAssertEqual(title.value as? String, "Reviewed version locally")
  }
}

extension EditorFlowTests {
  func testUnansweredPublicationKeepsEditingInAWorkingCopyAndWarnsBeforeSignOut() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(
        title: "Unanswered publication", description: "The earlier review may be on the server"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Reviewed photo")
    photo.serverId = UUID()
    photo.transfer = .accepted
    photo.variant = .ready
    let intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: Date())
    var publish = OutboxOperation(
      accountId: fixture.workspace.account.id, generation: 0,
      voyageId: voyage.id, kind: .publish(intentId: intent.id))
    publish.state = .uncertain
    publish.request = MutationRequest(method: "POST", path: "/possibly-accepted-publication")
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    fixture.workspace.publicationIntents = [intent]
    fixture.workspace.operations = [publish]
    fixture.reply("/auth/native/logout", body: Data(), status: 204)
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    XCTAssertTrue(
      app.staticTexts[
        "Checking whether publication was scheduled. The reviewed version may publish while this device is offline."
      ].waitForExistence(timeout: 5))
    app.buttons["Edit details"].tap()
    let title = app.textFields["Voyage name"]
    title.tap()
    title.typeText(" locally")
    let save = app.buttons["Save working copy on this iPhone"]
    app.reveal(save)
    save.tap()
    XCTAssertTrue(app.staticTexts[voyage.fields.title].waitForExistence(timeout: 5))
    app.reveal(app.buttons["Add photos"])
    XCTAssertFalse(app.buttons["Add photos"].isEnabled)
    app.buttons["Account"].tap()
    app.buttons["Sign out"].tap()
    app.buttons["Leave publication scheduled and sign out"].tap()
    XCTAssertTrue(app.staticTexts["Sign in to your account"].waitForExistence(timeout: 5))
  }
}
