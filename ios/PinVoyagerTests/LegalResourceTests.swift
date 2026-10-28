import XCTest

@testable import PinVoyager

@MainActor final class LegalResourceTests: XCTestCase {
  func testBundledPoliciesHaveTheirDraftRevisionAttributionAndOfflineText() async throws {
    let documents = try await BundledLegal.documents()
    XCTAssertEqual(Set(documents.map(\.id)), ["privacy", "terms"])
    for document in documents {
      XCTAssertEqual(document.revision, "2026-10-18")
      XCTAssertFalse(document.text.contains("{children}"))
      XCTAssertTrue(document.text.contains("Draft for review"))
      XCTAssertTrue(document.text.contains("CC BY-SA 3.0"))
      XCTAssertTrue(document.text.contains("inquiries@bryanporter.com"))
      XCTAssertTrue(document.text.contains("https://creativecommons.org/licenses/by-sa/3.0/"))
    }
    do {
      _ = try await BundledLegal.documents(bundle: Bundle(for: LegalResourceTests.self))
      XCTFail("Expected unavailable bundled documents")
    } catch { XCTAssertTrue(error is CocoaError) }
  }
}
