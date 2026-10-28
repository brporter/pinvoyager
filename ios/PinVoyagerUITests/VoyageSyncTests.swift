import PinVoyagerDomain
import XCTest

@MainActor final class VoyageSyncTests: XCTestCase {
  func testVoyageUploadPauseIsIndependentOfTheAccountPolicy() throws {
    var fixture = try FixtureLaunch()
    let voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Pause this voyage", description: "Keep other work running"))
    fixture.workspace.voyages = [voyage]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    app.reveal(app.buttons["Sync details"])
    app.buttons["Sync details"].tap()
    let pause = app.buttons["Pause uploads for voyage"]
    XCTAssertTrue(pause.waitForExistence(timeout: 3))
    guard pause.exists else { return }
    pause.tap()
    XCTAssertTrue(app.buttons["Resume uploads for voyage"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.staticTexts[
        "Uploads paused for this voyage. Receipt checks and other changes can still synchronize."
      ].exists)
    app.buttons["Sync and storage settings"].tap()
    XCTAssertEqual(app.switches["Pause synchronization"].value as? String, "0")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Resume uploads for voyage"].tap()
    XCTAssertTrue(pause.waitForExistence(timeout: 5))
  }
}

extension VoyageSyncTests {
  func testSyncGroupsVoyageActivityAndKeepsOnlyRecentCompletionsVisible() throws {
    var fixture = try FixtureLaunch()
    let voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Activity voyage", description: "Grouped by current state"))
    fixture.workspace.voyages = [voyage]
    for (state, kind) in [
      (OperationState.attention, OperationKind.edit(voyage.fields)),
      (.uncertain, .upload(photoId: UUID(), publicConsent: false)), (.waiting, .order([])),
      (.applied, .unpublish), (.applied, .deleteVoyage),
    ] {
      var operation = OutboxOperation(
        accountId: fixture.workspace.account.id, generation: 0,
        voyageId: voyage.id, kind: kind)
      operation.state = state
      if state == .applied {
        operation.completedAt =
          if case .deleteVoyage = kind { Date().addingTimeInterval(-8 * 86_400) } else { Date() }
      }
      fixture.workspace.operations.append(operation)
    }
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    XCTAssertTrue(app.staticTexts["Needs attention"].waitForExistence(timeout: 5))
    app.reveal(app.staticTexts["In progress"])
    XCTAssertTrue(app.staticTexts["In progress"].exists)
    app.reveal(app.staticTexts["Completed"])
    XCTAssertTrue(app.staticTexts["Completed"].exists)
    XCTAssertTrue(app.staticTexts["Make voyage private"].exists)
    XCTAssertFalse(app.staticTexts["Delete voyage"].exists)
    let open = app.buttons["sync.voyage.\(voyage.id)"]
    app.reveal(open.firstMatch, swipeUp: false)
    open.firstMatch.tap()
    XCTAssertTrue(app.navigationBars["Voyage sync details"].waitForExistence(timeout: 5))
  }
}

extension VoyageSyncTests {
  func testGlobalUploadPauseKeepsSynchronizationEnabledAndCanBeResumed() throws {
    let fixture = try FixtureLaunch()
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    app.buttons["Pause uploads"].tap()
    XCTAssertTrue(app.buttons["Resume uploads"].waitForExistence(timeout: 5))
    app.buttons["Sync and storage settings"].tap()
    XCTAssertEqual(app.switches["Pause photo uploads"].value as? String, "1")
    XCTAssertEqual(app.switches["Pause synchronization"].value as? String, "0")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Resume uploads"].tap()
    XCTAssertTrue(app.buttons["Pause uploads"].waitForExistence(timeout: 5))
  }
}
