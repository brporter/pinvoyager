import PinVoyagerData
import PinVoyagerDomain
import UIKit
import XCTest

@testable import PinVoyager

private struct PresentationPhotoSource: PhotoAcquiring {
  let filename: String
  let bytes: Data?
  func acquire() throws -> URL {
    guard let bytes else { throw FileVaultError.missingSource }
    let file = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      .appendingPathExtension("jpg")
    try bytes.write(to: file)
    return file
  }
}

@MainActor final class PhotoImportPresentationTests: XCTestCase {
  func testImportProgressKeepsFailuresAndReportsSavedOnlyAfterTheLocalCopyIsDurable() async throws {
    let state = workspaceWithPhotos()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let service = PhotoImportService(store: store, vault: try FileVault(root: root))
    let presentation = PhotoImportPresentation(service: service)
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 40, height: 40)).jpegData(
      withCompressionQuality: 0.8
    ) {
      UIColor.systemTeal.setFill()
      $0.fill(CGRect(x: 0, y: 0, width: 40, height: 40))
    }
    let sources = [
      PresentationPhotoSource(filename: "Unavailable.jpg", bytes: nil),
      PresentationPhotoSource(filename: "Saved.jpg", bytes: bytes),
    ]
    var changes = 0
    await presentation.importPhotos(
      sources, accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 1_000_000,
      publicConsent: false, cellularException: true
    ) {
      XCTAssertTrue(presentation.preparing)
      changes += 1
    }
    XCTAssertFalse(presentation.preparing)
    XCTAssertEqual(presentation.progress, "Saving 2 of 2 on this iPhone")
    XCTAssertEqual(presentation.failures.count, 1)
    XCTAssertGreaterThan(changes, 2)
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertTrue(
      saved.photos.contains {
        $0.filename == "Saved.jpg" && $0.source != nil && $0.cellularException == true
      })
    XCTAssertTrue(
      saved.photos.contains { $0.filename == "Unavailable.jpg" && $0.transfer == .needsSource })
    await presentation.importPhotos(
      [], accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 1,
      publicConsent: false
    ) {}
    XCTAssertEqual(presentation.failures.count, 1)
  }
}

extension PhotoImportPresentationTests {
  func testStoppingAnImportKeepsTheCurrentCopyAndRejectsAnotherConcurrentBatch() async throws {
    let state = workspaceWithPhotos()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let presentation = PhotoImportPresentation(
      service: PhotoImportService(store: store, vault: try FileVault(root: root)))
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 20, height: 20)).jpegData(
      withCompressionQuality: 0.8
    ) {
      UIColor.systemTeal.setFill()
      $0.fill(CGRect(x: 0, y: 0, width: 20, height: 20))
    }
    var stopped = false
    let account = state.account.id
    let voyage = state.voyages[0].id
    await presentation.importPhotos(
      [
        PresentationPhotoSource(filename: "Keep.jpg", bytes: bytes),
        PresentationPhotoSource(filename: "Skipped.jpg", bytes: bytes),
      ], accountId: account, voyageId: voyage, maxBytes: 1_000_000, publicConsent: false
    ) {
      if !stopped {
        stopped = true
        await presentation.importPhotos(
          [PresentationPhotoSource(filename: "Concurrent.jpg", bytes: bytes)], accountId: account,
          voyageId: voyage, maxBytes: 1_000_000, publicConsent: false
        ) {}
        await presentation.stop()
      }
    }
    let saved = try await store.read(accountId: account)
    XCTAssertTrue(saved.photos.contains { $0.filename == "Keep.jpg" && $0.source != nil })
    XCTAssertTrue(
      saved.photos.contains { $0.filename == "Skipped.jpg" && $0.transfer == .needsSource })
    XCTAssertFalse(saved.photos.contains { $0.filename == "Concurrent.jpg" })
    XCTAssertEqual(presentation.failures.count, 1)
    XCTAssertFalse(presentation.preparing)
  }

  func testPublicImportRequiresConsentBeforeAnyAcquisition() async throws {
    var state = workspaceWithPhotos()
    state.voyages[0].confirmedPublished = true
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let presentation = PhotoImportPresentation(
      service: PhotoImportService(store: store, vault: try FileVault(root: root)))
    await presentation.importPhotos(
      [PresentationPhotoSource(filename: "No consent.jpg", bytes: nil)],
      accountId: state.account.id, voyageId: state.voyages[0].id, maxBytes: 1_000_000,
      publicConsent: false
    ) {}
    XCTAssertFalse(presentation.preparing)
    XCTAssertEqual(presentation.failures.count, 1)
    let saved = try await store.read(accountId: state.account.id)
    XCTAssertFalse(saved.photos.contains { $0.filename == "No consent.jpg" })
  }
}
