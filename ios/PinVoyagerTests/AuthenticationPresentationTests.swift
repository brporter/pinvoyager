import AuthenticationServices
import PinVoyagerDomain
import UIKit
import XCTest

@testable import PinVoyager
@testable import PinVoyagerData

@MainActor private final class PresentationBrowser: BrowserAuthenticating {
  var opened: [(URL, Bool)] = []
  var action: (URL) async throws -> URL
  init(action: @escaping (URL) async throws -> URL) { self.action = action }
  func open(_ url: URL, ephemeral: Bool) async throws -> URL {
    opened.append((url, ephemeral))
    return try await action(url)
  }
}

@MainActor final class AuthenticationPresentationTests: XCTestCase {
  private func start(_ id: UUID) -> HTTPResponse {
    response(
      201,
      "{\"attempt_id\":\"\(id)\",\"browser_start_url\":\"https://pinvoyager.com/auth/native/attempts/\(id)/start\"}"
    )
  }
  private func callback(_ transport: StubHTTP) async throws -> URL {
    let requests = await transport.requests
    let request = try XCTUnwrap(requests.first)
    let body = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: request.httpBody!) as? [String: String])
    return try XCTUnwrap(
      URL(string: "pinvoyager://auth/callback?code=returned-code&state=\(body["state"]!)"))
  }
  func testSignInAndLinkingUseTheBrowserPreferenceAndAcceptOnlyACompletedExchange() async throws {
    for linking in [false, true] {
      let account = UUID()
      let attempt = UUID()
      let credentials = TestCredentials()
      await credentials.save(SessionCredential(accountId: account, token: "previous"))
      let transport = StubHTTP([
        start(attempt),
        response(
          200,
          "{\"session_token\":\"new-session\",\"user\":{\"id\":\"\(account)\",\"display_name\":\"traveler\"},\"identities\":[],\"idle_deadline\":\"2026-11-01T00:00:00Z\",\"absolute_deadline\":\"2026-12-01T00:00:00Z\"}"
        ),
      ])
      let auth = NativeAuthentication(
        client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
        credentials: credentials)
      let browser = PresentationBrowser { _ in try await self.callback(transport) }
      let presentation = AuthenticationPresentation(authentication: auth, browser: browser)
      var accepted = false
      let completed = await presentation.signIn(
        provider: "google", linkingAccountId: linking ? account : nil, ephemeral: true
      ) { session in
        XCTAssertTrue(presentation.busy)
        XCTAssertEqual(session.user.id, account)
        let saved = await credentials.read(accountId: account)
        XCTAssertEqual(saved?.token, "new-session")
        accepted = true
      }
      XCTAssertTrue(completed)
      XCTAssertTrue(accepted)
      XCTAssertFalse(presentation.busy)
      XCTAssertNil(presentation.error)
      XCTAssertEqual(browser.opened.first?.1, true)
      let requests = await transport.requests
      let body = try XCTUnwrap(
        try JSONSerialization.jsonObject(with: requests[0].httpBody!) as? [String: String])
      XCTAssertEqual(body["mode"], linking ? "link" : "sign_in")
    }
  }
  func testCancellationClearsTheAttemptAndAConcurrentButtonPressCannotCreateAnotherFlow()
    async throws
  {
    let attempt = UUID()
    let credentials = TestCredentials()
    let transport = StubHTTP([start(attempt)])
    let auth = NativeAuthentication(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials)
    let browser = PresentationBrowser { _ in throw AuthenticationError.canceled }
    let presentation = AuthenticationPresentation(authentication: auth, browser: browser)
    browser.action = { _ in
      let concurrent = await presentation.signIn(provider: "microsoft", ephemeral: false) { _ in
        XCTFail("Concurrent acceptance")
      }
      XCTAssertFalse(concurrent)
      throw ASWebAuthenticationSessionError(.canceledLogin)
    }
    let result = await presentation.signIn(provider: "apple", ephemeral: false) { _ in
      XCTFail("Canceled acceptance")
    }
    XCTAssertFalse(result)
    XCTAssertFalse(presentation.busy)
    XCTAssertNil(presentation.error)
    let requests = await transport.requests
    XCTAssertEqual(requests.count, 1)
    let body = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: requests[0].httpBody!) as? [String: String])
    let pending = PendingAuthentication(
      id: attempt, state: body["state"]!, verifier: "unused", browserStartUrl: browser.opened[0].0,
      linkingAccountId: nil, deletionAccountId: nil)
    do {
      _ = try await auth.complete(try await callback(transport), pending: pending)
      XCTFail("A canceled attempt must not exchange")
    } catch is AuthenticationError {} catch { XCTFail("Unexpected error: \(error)") }
  }
  func testBrowserFailuresRemainVisibleAndTheSpinnerStops() async throws {
    let credentials = TestCredentials()
    let transport = StubHTTP([start(UUID())])
    let auth = NativeAuthentication(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials)
    let browser = PresentationBrowser { _ in throw AuthenticationError.invalidAttempt }
    let presentation = AuthenticationPresentation(authentication: auth, browser: browser)
    let completed = await presentation.signIn(provider: "google", ephemeral: false) { _ in
      XCTFail("Failed acceptance")
    }
    XCTAssertFalse(completed)
    XCTAssertNotNil(presentation.error)
    XCTAssertFalse(presentation.busy)
    let invalid = await presentation.signIn(provider: "password", ephemeral: false) { _ in
      XCTFail("Unsupported provider")
    }
    XCTAssertFalse(invalid)
    XCTAssertNotNil(presentation.error)
  }
  func testACompletedExchangeStillReportsFailureWhenSavingTheLocalWorkspaceFails() async throws {
    let credentials = TestCredentials()
    let exchange = try Data(
      contentsOf: Fixtures.directory("api").appendingPathComponent("native_session_exchange.json"))
    let transport = StubHTTP([start(UUID()), HTTPResponse(status: 200, body: exchange)])
    let auth = NativeAuthentication(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials)
    let presentation = AuthenticationPresentation(
      authentication: auth,
      browser: PresentationBrowser { _ in try await self.callback(transport) })
    let completed = await presentation.signIn(provider: "google", ephemeral: false) { _ in
      throw CocoaError(.fileWriteOutOfSpace)
    }
    XCTAssertFalse(completed)
    XCTAssertFalse(presentation.busy)
    XCTAssertNotNil(presentation.error)
  }

  func testDeletionReauthenticationDeliversAPurposeBoundProofWithoutReplacingTheSession()
    async throws
  {
    let account = UUID()
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: account, token: "existing-session"))
    let transport = StubHTTP([
      start(UUID()),
      response(
        200,
        "{\"proof_token\":\"deletion-proof\",\"account_id\":\"\(account)\",\"purpose\":\"account_deletion\",\"provider\":\"apple\",\"expires_at\":\"2026-11-01T00:00:00Z\"}"
      ),
    ])
    let auth = NativeAuthentication(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials)
    let browser = PresentationBrowser { _ in try await self.callback(transport) }
    let presentation = AuthenticationPresentation(authentication: auth, browser: browser)
    var proof: String?
    let completed = await presentation.reauthenticate(provider: "apple", accountId: account) {
      proof = $0.proofToken
    }
    XCTAssertTrue(completed)
    XCTAssertEqual(proof, "deletion-proof")
    XCTAssertEqual(browser.opened.first?.1, false)
    let saved = await credentials.read(accountId: account)
    XCTAssertEqual(saved?.token, "existing-session")
    XCTAssertFalse(presentation.busy)
  }
}

@MainActor private final class StubAuthenticationSession: AuthenticationBrowserSession {
  var prefersEphemeralWebBrowserSession = false
  let startAction: () -> Bool
  init(start: @escaping () -> Bool) { startAction = start }
  func start() -> Bool { startAction() }
}

extension AuthenticationPresentationTests {
  func testSystemBrowserUsesItsWindowAndCompletesOnlyOnceWhenStartAndCallbacksRace() async throws {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    let url = URL(string: "https://pinvoyager.com/start")!
    let callback = URL(string: "pinvoyager://auth/callback?code=one&state=two")!
    var created: StubAuthenticationSession?
    let browser = BrowserAuthentication(
      anchor: { window },
      factory: { _, _, complete in
        let session = StubAuthenticationSession {
          complete(callback, nil)
          complete(nil, AuthenticationError.canceled)
          return false
        }
        created = session
        return session
      })
    let result = try await browser.open(url, ephemeral: true)
    XCTAssertEqual(result, callback)
    XCTAssertEqual(created?.prefersEphemeralWebBrowserSession, true)
    let anchorSession = ASWebAuthenticationSession(url: url, callbackURLScheme: "pinvoyager") {
      _, _ in
    }
    XCTAssertTrue(browser.presentationAnchor(for: anchorSession) === window)
    let cannotStart = BrowserAuthentication(
      anchor: { window },
      factory: { _, _, _ in
        StubAuthenticationSession { false }
      })
    do {
      _ = try await cannotStart.open(url, ephemeral: false)
      XCTFail("A browser that cannot start must fail")
    } catch is AuthenticationError {} catch { XCTFail("Unexpected error: \(error)") }
    let canceled = BrowserAuthentication(
      anchor: { window },
      factory: { _, _, complete in
        StubAuthenticationSession {
          complete(nil, nil)
          return true
        }
      })
    do {
      _ = try await canceled.open(url, ephemeral: false)
      XCTFail("A missing callback must report cancellation")
    } catch is AuthenticationError {} catch { XCTFail("Unexpected error: \(error)") }
  }
}
