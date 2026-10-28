import PinVoyagerData
import PinVoyagerDomain
import XCTest

@MainActor final class SyncFlowTests: XCTestCase {
  func testSyncSettingsAndExplicitMetadataConflictChoices() throws {
    var fixture = try FixtureLaunch()
    let account = fixture.workspace.account.id
    let fields = VoyageFields(
      title: "My retained title", description: "My description", granularity: .exact)
    var voyage = LocalVoyage(ownerId: account, fields: fields)
    var remote = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_updated"))
        as? [String: Any])
    remote["owner_id"] = account.uuidString
    voyage.serverId = UUID(uuidString: try XCTUnwrap(remote["id"] as? String))
    voyage.serverFields = VoyageFields(
      title: "Original", description: "Original", granularity: .km1)
    voyage.revision = "1"
    fixture.workspace.voyages = [voyage]
    var edit = OutboxOperation(
      accountId: account, generation: 0, voyageId: voyage.id, kind: .edit(fields),
      expectedRevision: "1")
    edit.state = .conflict
    edit.lastError = "Review the website changes."
    edit.requestId = "conflict-reference"
    fixture.workspace.operations = [edit]
    fixture.reply(
      "/api/voyages/\(voyage.serverId!)", body: try JSONSerialization.data(withJSONObject: remote))
    let photos = (remote["photos"] as? [[String: Any]] ?? []).map { photo in
      [
        "id": photo["id"]!, "width": photo["width"]!, "height": photo["height"]!,
        "variant_status": photo["variant_status"]!,
      ]
    }
    fixture.reply(
      "/api/voyages/\(voyage.serverId!)/photos",
      body: try JSONSerialization.data(withJSONObject: ["photos": photos, "next_before": NSNull()]))
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    XCTAssertTrue(app.staticTexts["Reference: conflict-reference"].waitForExistence(timeout: 5))
    app.buttons["Sync and storage settings"].tap()
    app.switches["Upload photos on Wi-Fi only"].tap()
    app.switches["Use Low Data Mode connections"].tap()
    app.switches["Allow scheduled background preparation"].tap()
    app.switches["Pause synchronization"].tap()
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Resolve changes"].tap()
    XCTAssertTrue(app.staticTexts["Your saved version"].firstMatch.waitForExistence(timeout: 5))
    for field in ["Voyage name", "Description", "Public location precision"] {
      app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", field)).firstMatch.tap()
      app.buttons["Use mine"].tap()
    }
    app.reveal(app.buttons["Save resolution"])
    app.buttons["Save resolution"].tap()
    XCTAssertTrue(app.navigationBars["Sync"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Resolve changes"].exists)
    app.buttons["My Voyages"].tap()
    XCTAssertTrue(app.staticTexts["My retained title"].waitForExistence(timeout: 5))
  }

  func testUnavailableConflictCanBecomeANewPrivateDraft() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Recovered destination", description: "Kept locally"))
    voyage.serverId = UUID()
    voyage.revision = "1"
    fixture.workspace.voyages = [voyage]
    var edit = OutboxOperation(
      accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
      kind: .edit(voyage.fields))
    edit.state = .conflict
    fixture.workspace.operations = [edit]
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    app.buttons["Resolve changes"].tap()
    let recovery = app.buttons["Keep as a new private voyage"]
    XCTAssertTrue(recovery.waitForExistence(timeout: 5))
    recovery.tap()
    app.buttons["My Voyages"].tap()
    XCTAssertEqual(app.staticTexts.matching(identifier: "Recovered destination").count, 2)
  }
}

extension SyncFlowTests {
  func testEveryPendingOperationHasAnActionableLabelAndCanBeRetried() throws {
    var fixture = try FixtureLaunch()
    let account = fixture.workspace.account.id
    let voyage = UUID()
    let fields = VoyageFields(title: "Pending voyage", description: "Retained on device")
    let kinds: [(String, OperationKind)] = [
      ("Create draft", .create(fields)), ("Save voyage details", .edit(fields)),
      ("Upload photograph", .upload(photoId: UUID(), publicConsent: false)),
      ("Save photo order", .order([])),
      ("Remove photograph", .removePhoto(photoId: UUID(), uploadKey: UUID())),
      ("Delete voyage", .deleteVoyage),
      ("Make voyage private", .unpublish), ("Request publication", .publish(intentId: UUID())),
      ("Cancel publication", .cancelPublication(intentId: UUID())),
      ("Update blocked traveler", .block(userId: UUID(), blocked: true)),
      ("Send report", .report(target: "voyage:\(UUID())", reason: "spam", details: "Review this")),
    ]
    for (index, entry) in kinds.enumerated() {
      // Keep Create draft just below the first viewport, including its prefetched Retry button.
      let ordinal = index == 0 ? 3 : index < 3 ? index : index + 1
      let id = try XCTUnwrap(
        UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", ordinal)))
      var operation = OutboxOperation(
        id: id, accountId: account, generation: 0, voyageId: voyage, kind: entry.1)
      operation.state = .attention
      operation.lastError = "Kept locally. Review this operation before retrying."
      fixture.workspace.operations.append(operation)
    }
    let retryId = fixture.workspace.operations[0].id
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    var found: Set<String> = []
    for _ in 0..<10 {
      for (title, _) in kinds where app.staticTexts[title].exists { found.insert(title) }
      app.swipeUp()
    }
    XCTAssertEqual(found, Set(kinds.map(\.0)))
    let retry = app.buttons["sync.retry.\(retryId)"]
    for _ in 0..<12 {
      if retry.exists { break }
      app.swipeDown()
    }
    XCTAssertTrue(retry.exists)
    let content = app.frame.insetBy(dx: 0, dy: app.frame.height / 4)
    app.reveal(retry, swipeUp: false, within: content)
    XCTAssertTrue(content.contains(retry.frame), "Retry must be clear of navigation and tab bars.")
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = "Retry before tapping: \(retry.frame)"
    screenshot.lifetime = .keepAlways
    add(screenshot)
    retry.tap()
    XCTAssertTrue(retry.waitForNonExistence(timeout: 5), app.debugDescription)
  }
}

extension SyncFlowTests {
  func testPhotoMembershipConflictShowsBothOrdersAndKeepsWebsiteDeletions() throws {
    var fixture = try FixtureLaunch()
    var detail = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_updated"))
        as? [String: Any])
    detail["owner_id"] = fixture.workspace.account.id.uuidString
    detail["published"] = false
    let bytes = try JSONSerialization.data(withJSONObject: detail)
    let remote = try APIJSON.decoder().decode(RemoteVoyage.self, from: bytes)
    var voyage = LocalVoyage(ownerId: fixture.workspace.account.id, fields: remote.fields)
    voyage.serverId = remote.id
    voyage.serverFields = remote.fields
    voyage.revision = "1"
    fixture.workspace.voyages = [voyage]
    for (index, remotePhoto) in remote.photos.prefix(2).enumerated() {
      var photo = LocalPhoto(voyageId: voyage.id, filename: "Retained website photo \(index + 1)")
      photo.serverId = remotePhoto.id
      photo.transfer = .accepted
      photo.variant = .ready
      photo.order = index
      fixture.workspace.photos.append(photo)
    }
    var removed = LocalPhoto(voyageId: voyage.id, filename: "Deleted while offline.jpg")
    removed.serverId = UUID()
    removed.source = SourceFile(
      relativePath: "retained.jpg", contentHash: String(repeating: "a", count: 64), byteCount: 10,
      mediaType: "image/jpeg")
    removed.order = 2
    fixture.workspace.photos.append(removed)
    var added = LocalPhoto(voyageId: voyage.id, filename: "Added on my iPhone.jpg")
    added.source = removed.source
    added.order = 3
    added.transfer = .saved
    fixture.workspace.photos.append(added)
    var conflict = OutboxOperation(
      accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
      kind: .order(fixture.workspace.photos.map(\.id)), expectedRevision: "1")
    conflict.state = .conflict
    fixture.workspace.operations = [
      conflict,
      OutboxOperation(
        accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
        kind: .upload(photoId: added.id, publicConsent: false)),
    ]
    fixture.reply("/api/voyages/\(remote.id)", body: bytes)
    let metadata = remote.photos.map {
      ["id": $0.id.uuidString, "width": $0.width, "height": $0.height, "variant_status": "ready"]
        as [String: Any]
    }
    fixture.reply(
      "/api/voyages/\(remote.id)/photos",
      body: try JSONSerialization.data(withJSONObject: [
        "photos": metadata, "next_before": NSNull(),
      ]))
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    app.buttons["Resolve changes"].tap()
    let selection = app.buttons.matching(
      NSPredicate(format: "label BEGINSWITH %@", "Photo resolution")
    ).firstMatch
    for _ in 0..<8 {
      if selection.exists { break }
      app.swipeUp()
    }
    XCTAssertTrue(selection.waitForExistence(timeout: 5))
    app.reveal(selection)
    selection.tap()
    app.buttons["Use website order and append my new photos"].tap()
    let save = app.buttons["Save resolution"]
    for _ in 0..<6 {
      if save.exists { break }
      app.swipeUp()
    }
    app.reveal(save)
    XCTAssertTrue(save.isEnabled)
    save.tap()
    XCTAssertTrue(app.navigationBars["Sync"].waitForExistence(timeout: 5))
    app.buttons["My Voyages"].tap()
    app.staticTexts[remote.title].tap()
    app.reveal(app.staticTexts["Added on my iPhone.jpg"])
    XCTAssertTrue(app.staticTexts["Added on my iPhone.jpg"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.staticTexts["Deleted while offline.jpg"].exists)
    let count = app.staticTexts["4 photos · 4 without location"]
    app.reveal(count, swipeUp: false)
    XCTAssertTrue(count.exists, app.debugDescription)
  }
}

extension SyncFlowTests {
  func testUnavailableDestinationCanDiscardLocalWorkAfterConfirmation() throws {
    var fixture = try FixtureLaunch()
    var voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Discard unavailable voyage", description: "Explicitly discarded")
    )
    voyage.serverId = UUID()
    voyage.revision = "1"
    fixture.workspace.voyages = [voyage]
    var operation = OutboxOperation(
      accountId: fixture.workspace.account.id, generation: 0,
      voyageId: voyage.id, kind: .edit(voyage.fields))
    operation.state = .conflict
    fixture.workspace.operations = [operation]
    let app = try fixture.launch()
    app.buttons["Sync"].tap()
    app.buttons["Resolve changes"].tap()
    let discard = app.buttons["Discard local work"]
    XCTAssertTrue(discard.waitForExistence(timeout: 5))
    guard discard.exists else { return }
    discard.tap()
    app.buttons["Discard files and edits"].tap()
    XCTAssertTrue(app.navigationBars["Sync"].waitForExistence(timeout: 5))
    app.buttons["My Voyages"].tap()
    XCTAssertFalse(app.staticTexts[voyage.fields.title].exists)
  }

  func testDefinitelyLocalVoyageDeletionNamesTheDeviceAndUnsyncedCount() throws {
    var fixture = try FixtureLaunch()
    let voyage = LocalVoyage(
      ownerId: fixture.workspace.account.id,
      fields: VoyageFields(title: "Delete only this local voyage", description: "Not submitted"))
    fixture.workspace.voyages = [voyage]
    fixture.workspace.photos = [LocalPhoto(voyageId: voyage.id, filename: "Not uploaded")]
    let app = try fixture.launch()
    app.buttons["My Voyages"].tap()
    app.staticTexts[voyage.fields.title].swipeLeft()
    app.buttons["Delete"].tap()
    let delete = app.buttons["Delete from this iPhone"]
    XCTAssertTrue(delete.waitForExistence(timeout: 3))
    guard delete.exists else { return }
    XCTAssertTrue(
      app.staticTexts[
        "This removes 1 unsynced photos and their local files after stopping queued uploads."
      ].exists)
    delete.tap()
    XCTAssertTrue(app.staticTexts[voyage.fields.title].waitForNonExistence(timeout: 5))
  }
}

extension SyncFlowTests {
  func testOfflineConflictRestoresItsArrangedWorkingCopyAndSavesNewChoicesLocally() throws {
    var fixture = try FixtureLaunch()
    let fields = VoyageFields(
      title: "Offline conflict choices", description: "Local edits", granularity: .km10)
    var voyage = LocalVoyage(ownerId: fixture.workspace.account.id, fields: fields)
    voyage.serverId = UUID()
    voyage.revision = "1"
    fixture.workspace.voyages = [voyage]
    var removed = LocalPhoto(voyageId: voyage.id, filename: "Removed on website")
    removed.serverId = UUID()
    removed.source = SourceFile(
      relativePath: "originals/retained", contentHash: String(repeating: "a", count: 64),
      byteCount: 12, mediaType: "image/jpeg")
    var local = LocalPhoto(voyageId: voyage.id, filename: "Local arrival")
    local.source = removed.source
    local.order = 1
    fixture.workspace.photos = [removed, local]
    var operation = OutboxOperation(
      accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
      kind: .order([removed.id, local.id]))
    operation.state = .conflict
    fixture.workspace.operations = [operation]
    let website = UUID()
    var review = ConflictReview(
      voyageId: voyage.id, generation: 0, mine: fields,
      website: VoyageFields(
        title: "Website fields", description: "Previously fetched", granularity: .km1),
      revision: "7", published: true, websitePhotos: [website])
    review.choices = ["title": .local, "description": .remote, "granularity": .local]
    review.photoChoice = .arranged([.local(local.id), .website(website)])
    review.readded = [removed.id]
    fixture.workspace.conflictReviews = [review]
    let app = try fixture.launch(arguments: ["--offline-fixture"])
    app.buttons["Sync"].tap()
    app.buttons["Resolve changes"].tap()
    let selection = app.buttons.matching(
      NSPredicate(format: "label BEGINSWITH %@", "Photo resolution")
    ).firstMatch
    app.reveal(selection)
    selection.tap()
    app.buttons["Keep website version"].tap()
    app.reveal(selection, swipeUp: false)
    selection.tap()
    app.buttons["Arrange combined photos"].tap()
    let added = app.buttons["Added with a new upload identity"]
    app.reveal(added)
    XCTAssertTrue(added.exists)
    XCTAssertFalse(added.isEnabled)
    app.reveal(app.buttons["Save resolution on this iPhone"], swipeUp: false)
    app.buttons["Save resolution on this iPhone"].tap()
    XCTAssertTrue(app.staticTexts["Resolution saved on this iPhone"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Resolve changes"].tap()
    app.reveal(selection)
    XCTAssertTrue(selection.label.contains("Arrange combined photos"))
  }

  func testVisibilityConflictCanKeepWebsiteVisibilityOrRequestUnpublication() throws {
    for choice in ["Keep website visibility", "Make voyage private"] {
      var fixture = try FixtureLaunch()
      var detail = try XCTUnwrap(
        try JSONSerialization.jsonObject(with: FixtureLaunch.fixture("voyage_updated"))
          as? [String: Any])
      detail["owner_id"] = fixture.workspace.account.id.uuidString
      detail["published"] = true
      let bytes = try JSONSerialization.data(withJSONObject: detail)
      let remote = try APIJSON.decoder().decode(RemoteVoyage.self, from: bytes)
      var voyage = LocalVoyage(ownerId: fixture.workspace.account.id, fields: remote.fields)
      voyage.serverId = remote.id
      voyage.revision = "1"
      voyage.confirmedPublished = true
      fixture.workspace.voyages = [voyage]
      var operation = OutboxOperation(
        accountId: fixture.workspace.account.id, generation: 0, voyageId: voyage.id,
        kind: .unpublish)
      operation.state = .conflict
      fixture.workspace.operations = [operation]
      fixture.reply("/api/voyages/\(remote.id)", body: bytes)
      let metadata = remote.photos.map {
        ["id": $0.id.uuidString, "width": $0.width, "height": $0.height, "variant_status": "ready"]
          as [String: Any]
      }
      fixture.reply(
        "/api/voyages/\(remote.id)/photos",
        body: try JSONSerialization.data(withJSONObject: [
          "photos": metadata, "next_before": NSNull(),
        ]))
      let app = try fixture.launch()
      app.buttons["Sync"].tap()
      app.buttons["Resolve changes"].tap()
      let selection = app.buttons.matching(
        NSPredicate(format: "label BEGINSWITH %@", "Visibility resolution")
      ).firstMatch
      XCTAssertTrue(selection.waitForExistence(timeout: 5))
      XCTAssertFalse(app.buttons["Save resolution"].isEnabled)
      selection.tap()
      app.buttons[choice].tap()
      app.buttons["Save resolution"].tap()
      XCTAssertTrue(app.navigationBars["Sync"].waitForExistence(timeout: 5))
      XCTAssertFalse(app.buttons["Resolve changes"].exists)
      app.buttons["My Voyages"].tap()
      XCTAssertTrue(app.staticTexts["Public"].exists)
      app.terminate()
    }
  }
}
