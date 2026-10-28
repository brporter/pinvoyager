import PinVoyagerData
import PinVoyagerDomain
import XCTest

@testable import PinVoyager

@MainActor final class ProfilePresentationTests: XCTestCase {
  private let userId = UUID()
  private let access = PublicAccess(
    accountId: nil, generation: nil, validation: UUID(), offline: false, blocked: [],
    pendingBlocks: false)
  private func page(cursor: String?) throws -> Data {
    let list = try XCTUnwrap(
      try JSONSerialization.jsonObject(
        with: Data(contentsOf: Fixtures.directory("api").appendingPathComponent("voyage_list.json"))
      ) as? [String: Any])
    return try JSONSerialization.data(withJSONObject: [
      "user": ["id": userId.uuidString, "display_name": "traveler"], "voyages": list["voyages"]!,
      "next_before": cursor as Any? ?? NSNull(),
    ])
  }
  func testFailedPaginationKeepsExistingCardsAndRetryUsesTheSameCursor() async throws {
    let transport = StubHTTP([
      HTTPResponse(status: 200, body: try page(cursor: "next-page")), response(503, ""),
      HTTPResponse(status: 200, body: try page(cursor: nil)),
    ])
    let presentation = ProfilePresentation(
      identifier: "traveler",
      client: APIClient(
        origin: try testOrigin(), transport: transport, credentials: TestCredentials()))
    await presentation.load(refresh: true, access: access) { $0 == self.access }
    let ids = presentation.voyages.map(\.id)
    XCTAssertFalse(ids.isEmpty)
    await presentation.load(refresh: false, access: access) { $0 == self.access }
    XCTAssertEqual(presentation.voyages.map(\.id), ids)
    XCTAssertEqual(presentation.cursor, "next-page")
    XCTAssertNotNil(presentation.error)
    XCTAssertTrue(presentation.retryingMore)
    await presentation.retry(access: access) { $0 == self.access }
    XCTAssertEqual(presentation.voyages.map(\.id), ids)
    XCTAssertNil(presentation.cursor)
    XCTAssertNil(presentation.error)
    XCTAssertFalse(presentation.loading)
    let requests = await transport.requests
    XCTAssertEqual(requests[1].url?.query, requests[2].url?.query)
  }
  func testDeniedRefreshAndOfflineStateClearPreviouslyLoadedProfileContent() async throws {
    let transport = StubHTTP([
      HTTPResponse(status: 200, body: try page(cursor: "next")), response(404, ""),
    ])
    let presentation = ProfilePresentation(
      identifier: "traveler",
      client: APIClient(
        origin: try testOrigin(), transport: transport, credentials: TestCredentials()))
    await presentation.load(refresh: true, access: access) { _ in true }
    XCTAssertNotNil(presentation.profile)
    await presentation.load(refresh: true, access: access) { _ in true }
    XCTAssertNil(presentation.profile)
    XCTAssertTrue(presentation.voyages.isEmpty)
    let offline = PublicAccess(
      accountId: nil, generation: nil, validation: UUID(), offline: true, blocked: [],
      pendingBlocks: false)
    await presentation.load(refresh: true, access: offline) { _ in true }
    XCTAssertNotNil(presentation.error)
    let requests = await transport.requests
    XCTAssertEqual(requests.count, 2)
  }
  func testLateProfileResultsCannotRestoreContentAfterTheViewersAccessChanges() async throws {
    let presentation = ProfilePresentation(
      identifier: "traveler",
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([
          HTTPResponse(status: 200, body: try page(cursor: nil)), response(503, ""),
        ]), credentials: TestCredentials()))
    await presentation.load(refresh: true, access: access) { _ in false }
    XCTAssertNil(presentation.profile)
    XCTAssertTrue(presentation.voyages.isEmpty)
    await presentation.load(refresh: true, access: access) { _ in false }
    XCTAssertNil(presentation.error)
    presentation.invalidate()
    XCTAssertNil(presentation.loadedAccess)
  }
}
