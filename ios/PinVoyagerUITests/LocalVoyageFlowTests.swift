import PinVoyagerData
import PinVoyagerDomain
import UIKit
import XCTest

@MainActor final class LocalVoyageFlowTests: XCTestCase {
  func testOwnerDownloadsGalleryRemovalAndUnpublicationUseConfirmedStates() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(
        title: "Owned public voyage", description: "A saved website copy", granularity: .km1))
    voyage.serverId = UUID()
    voyage.revision = "7"
    voyage.serverFields = voyage.fields
    voyage.confirmedPublished = true
    fixture.workspace.voyages = [voyage]
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Website photograph")
    photo.serverId = UUID()
    photo.transfer = .accepted
    photo.variant = .ready
    photo.coordinate = Coordinate(latitude: 35.123, longitude: 139.456)
    fixture.workspace.photos = [photo]
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 120, height: 80)).jpegData(
      withCompressionQuality: 0.8
    ) {
      UIColor.systemTeal.setFill()
      $0.fill(CGRect(x: 0, y: 0, width: 120, height: 80))
    }
    try fixture.provideOwnerDownload(voyage: voyage, photo: photo, bytes: bytes)
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    let locations = app.switches["Private locations (only you)"]
    XCTAssertTrue(locations.waitForExistence(timeout: 5))
    locations.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
    app.reveal(app.staticTexts["Exact recorded locations — private metadata"])
    XCTAssertTrue(app.staticTexts["Exact recorded locations — private metadata"].exists)
    let download = app.buttons["Download website copy"]
    app.reveal(download)
    XCTAssertTrue(download.isEnabled)
    download.tap()
    let downloaded = app.staticTexts["Website copy available offline"]
    XCTAssertTrue(downloaded.waitForExistence(timeout: 10))
    let photoRow = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", photo.filename))
      .firstMatch
    app.reveal(photoRow, swipeUp: false)
    photoRow.tap()
    XCTAssertTrue(app.staticTexts["Downloaded owner copy"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.staticTexts.matching(
        NSPredicate(format: "label BEGINSWITH %@", "Private recorded location:")
      ).firstMatch.exists)
    app.buttons["Close"].tap()
    let remove = app.buttons["Remove downloaded copy"]
    app.reveal(remove)
    remove.tap()
    app.buttons["Remove download"].tap()
    XCTAssertTrue(downloaded.waitForNonExistence(timeout: 5))
    let makePrivate = app.buttons["Make voyage private"]
    app.reveal(makePrivate)
    makePrivate.tap()
    try XCTUnwrap(
      app.buttons.matching(identifier: "Make voyage private").allElementsBoundByIndex.last {
        $0.isHittable
      }
    ).tap()
    app.buttons["Sync"].tap()
    XCTAssertTrue(app.staticTexts["Make voyage private"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Waiting"].firstMatch.exists)
  }
}

extension LocalVoyageFlowTests {
  func testArmedPublicationDisablesPhotoChangesUntilCancellationIsConfirmed() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Scheduled private voyage", description: "Waiting for processing")
    )
    voyage.serverId = UUID()
    voyage.revision = "7"
    var photo = LocalPhoto(voyageId: voyage.id, filename: "Processing photograph")
    photo.serverId = UUID()
    photo.transfer = .accepted
    photo.variant = .pending
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [photo]
    var intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: Date())
    intent.serverId = UUID()
    intent.status = .processing
    fixture.workspace.publicationIntents = [intent]
    var operation = OutboxOperation(
      accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
      kind: .publish(intentId: intent.id))
    operation.state = .applied
    fixture.workspace.operations = [operation]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    XCTAssertTrue(
      app.staticTexts[
        "Publication scheduled. The reviewed version may publish while this device is offline."
      ].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Edit details"].isEnabled)
    let add = app.buttons["Add photos"]
    app.reveal(add)
    XCTAssertFalse(add.isEnabled)
    XCTAssertTrue(app.staticTexts["Processing photo"].exists)
    let review = app.buttons["Review publication"]
    app.reveal(review)
    XCTAssertFalse(review.isEnabled)
    app.reveal(app.buttons["Sync details"])
    app.buttons["Sync details"].tap()
    app.reveal(app.buttons["Cancel publication"])
    app.buttons["Cancel publication"].tap()
    XCTAssertTrue(
      app.staticTexts["Cancellation is waiting for a connection. This voyage may still publish."]
        .waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Processing"].exists)
  }

  func testAddingToAPublicVoyageRequiresExplicitPhotoConsent() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Public photos need consent", description: "A public voyage"))
    voyage.serverId = UUID()
    voyage.revision = "1"
    voyage.confirmedPublished = true
    fixture.workspace.voyages = [voyage]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    app.reveal(app.buttons["Add photos"])
    app.buttons["Add photos"].tap()
    XCTAssertTrue(app.buttons["Choose photos"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Choose photos"].isEnabled)
    XCTAssertFalse(app.buttons["Take photo"].isEnabled)
    app.switches["New photos will become public as they upload"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)
    ).tap()
    XCTAssertTrue(app.buttons["Choose photos"].isEnabled)
    XCTAssertTrue(app.buttons["Take photo"].isEnabled)
    app.buttons["Close"].tap()
  }
}

extension LocalVoyageFlowTests {
  func testWebsiteOwnerGallerySupportsThumbnailsPrivateInformationZoomAndReturningToMap() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Owner gallery details", description: "Two website photos"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    voyage.confirmedPublished = true
    fixture.workspace.voyages = [voyage]
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 120, height: 80)).jpegData(
      withCompressionQuality: 0.8
    ) {
      UIColor.systemTeal.setFill()
      $0.fill(CGRect(x: 0, y: 0, width: 120, height: 80))
    }
    fixture.workspace.photos = (0..<2).map { index in
      var photo = LocalPhoto(voyageId: voyage.id, filename: "Website photo \(index + 1)")
      photo.serverId = UUID()
      photo.transfer = .accepted
      photo.variant = .ready
      photo.width = 120
      photo.height = 80
      photo.order = index
      if index == 0 {
        photo.coordinate = Coordinate(latitude: 35.123456, longitude: 139.456789)
        photo.capturedAt = "2026:10:01 12:30:00"
      }
      return photo
    }
    for photo in fixture.workspace.photos {
      for kind in ["thumb", "display", "full"] {
        fixture.reply("/api/photos/\(photo.serverId!)/\(kind)", body: bytes)
      }
    }
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].tap()
    let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Website photo 1"))
      .firstMatch
    app.reveal(row)
    row.tap()
    XCTAssertTrue(app.navigationBars["1 of 2"].waitForExistence(timeout: 5))
    assertGalleryFillsWindow(app, context: "owner photos")
    let image = app.images["Photograph. Pinch to zoom."].firstMatch
    XCTAssertTrue(image.waitForExistence(timeout: 5))
    image.pinch(withScale: 2, velocity: 2)
    image.doubleTap()
    app.buttons["Info"].tap()
    XCTAssertTrue(app.staticTexts["Exact location — private metadata"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.descendants(matching: .any).matching(
        NSPredicate(
          format: "label CONTAINS %@ OR value CONTAINS %@", "2026:10:01 12:30:00",
          "2026:10:01 12:30:00")
      ).firstMatch.exists, app.debugDescription)
    app.buttons["Done"].tap()
    app.buttons["Thumbnail 2"].tap()
    XCTAssertTrue(app.navigationBars["2 of 2"].waitForExistence(timeout: 5))
    app.buttons["Info"].tap()
    XCTAssertTrue(app.staticTexts["No recorded location"].waitForExistence(timeout: 5))
    app.buttons["Done"].tap()
    app.buttons["Thumbnail 1"].tap()
    app.buttons["Info"].tap()
    app.reveal(app.buttons["Show on map"])
    app.buttons["Show on map"].tap()
    XCTAssertTrue(app.buttons["Fit voyage"].waitForExistence(timeout: 5))
    let pin = app.buttons["View 1 photographs at this location"]
    app.reveal(pin, swipeUp: false)
    XCTAssertTrue(pin.waitForExistence(timeout: 10))
    pin.tap()
    assertGalleryFillsWindow(app, context: "owner map")
    app.buttons["Close"].tap()
    app.reveal(app.buttons["Share voyage"])
    XCTAssertTrue(app.buttons["Share voyage"].exists)
  }
}

extension LocalVoyageFlowTests {
  func testRemoteVoyageDeletionOffersUndoOnlyBeforeSubmission() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(
        title: "Undo remote deletion", description: "Website deletion has not been submitted"))
    voyage.serverId = UUID()
    voyage.revision = "7"
    fixture.workspace.voyages = [voyage]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].swipeLeft()
    app.buttons["Delete"].tap()
    app.buttons["Delete voyage and its photos"].tap()
    let undo = app.buttons["voyage.undo.\(voyage.id)"]
    XCTAssertTrue(undo.waitForExistence(timeout: 5))
    guard undo.exists else { return }
    undo.tap()
    XCTAssertTrue(app.staticTexts[voyage.fields.title].waitForExistence(timeout: 5))
    XCTAssertFalse(undo.exists)
  }
}
