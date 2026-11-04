import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test
func completedAuthenticationIsSingleUseAndStoresOnlyAppSession() async throws {
  let attemptId = UUID()
  let accountId = UUID()
  let credentials = TestCredentials()
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(attemptId)\",\"browser_start_url\":\"https://pinvoyager.com/auth/native/attempts/\(attemptId)/start\"}"
    ),
    response(
      200,
      "{\"session_token\":\"opaque-app-session\",\"user\":{\"id\":\"\(accountId)\",\"display_name\":\"alice\"},\"identities\":[],\"idle_deadline\":\"2026-11-01T00:00:00Z\",\"absolute_deadline\":\"2026-12-01T00:00:00Z\"}"
    ),
  ])
  let client = APIClient(origin: try testOrigin(), transport: transport, credentials: credentials)
  let authentication = NativeAuthentication(client: client, credentials: credentials)
  let pending = try await authentication.begin(provider: "google")
  let callback = URL(
    string: "pinvoyager://auth/callback?code=single-use-code&state=\(pending.state)"
  )!
  let reply = try await authentication.complete(callback, pending: pending)
  #expect(reply.user.id == accountId)
  #expect(await credentials.read(accountId: accountId)?.token == "opaque-app-session")
  await #expect(throws: AuthenticationError.self) {
    try await authentication.complete(callback, pending: pending)
  }
  #expect(await transport.requests.count == 2)
  let body = try #require(
    JSONSerialization.jsonObject(with: await transport.requests[0].httpBody!) as? [String: String]
  )
  #expect(body["mode"] == "sign_in")
  #expect(body["challenge"]?.count == 43)
  #expect(body["verifier"] == nil)
}

@Test
func revocationOnlyCredentialsCannotUploadAndCanRevokeThemselves() async throws {
  let accountId = UUID()
  let credentials = TestCredentials()
  var token = SessionCredential(accountId: accountId, token: "old-session")
  token.revocationOnly = true
  await credentials.save(token)
  let transport = StubHTTP([response(204, "")])
  let client = APIClient(origin: try testOrigin(), transport: transport, credentials: credentials)
  await #expect(throws: AuthenticationError.self) {
    let _: AccountResponse = try await client.me(accountId)
  }
  #expect(await transport.requests.isEmpty)
  try await NativeAuthentication(client: client, credentials: credentials)
    .revokePendingCredentials()
  #expect(await credentials.all().isEmpty)
  #expect(
    await transport.requests.first?.value(forHTTPHeaderField: "Authorization")
      == "Bearer old-session"
  )
}

@Test
func authenticationRefusesForeignStartURLsAndUnsupportedProviders() async throws {
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(UUID())\",\"browser_start_url\":\"https://evil.test/start\"}"
    )
  ])
  let auth = NativeAuthentication(
    client: APIClient(
      origin: try testOrigin(),
      transport: transport,
      credentials: TestCredentials()
    ),
    credentials: TestCredentials()
  )
  await #expect(throws: AuthenticationError.self) { try await auth.begin(provider: "password") }
  await #expect(throws: AuthenticationError.self) { try await auth.begin(provider: "google") }
}

@Test
func authenticationAcceptsCanonicalLowercaseServerUUIDs() async throws {
  let id = UUID()
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(id.uuidString.lowercased())\",\"browser_start_url\":\"https://pinvoyager.com/auth/native/attempts/\(id.uuidString.lowercased())/start\"}"
    )
  ])
  let auth = NativeAuthentication(
    client: APIClient(
      origin: try testOrigin(),
      transport: transport,
      credentials: TestCredentials()
    ),
    credentials: TestCredentials()
  )
  let pending = try await auth.begin(provider: "google")
  #expect(pending.id == id)
  await auth.cancel(pending)
}

#if os(iOS)
  @Test
  func keychainRoundTripAndRemovalUseAnIsolatedService() async throws {
    let keychain = KeychainCredentialStore(service: "com.pinvoyager.tests.\(UUID())")
    let accountId = UUID()
    #expect(try await keychain.read(accountId: accountId) == nil)
    #expect(try await keychain.all().isEmpty)
    var credential = SessionCredential(accountId: accountId, token: "opaque-session")
    try await keychain.save(credential)
    #expect(try await keychain.read(accountId: accountId) == credential)
    credential.revocationOnly = true
    try await keychain.save(credential)
    #expect(try await keychain.all() == [credential])
    try await keychain.remove(accountId: accountId)
    try await keychain.remove(accountId: accountId)
    #expect(try await keychain.read(accountId: accountId) == nil)
  }
#endif

@Test
func nativeAppleBindsStateAndNonceAndRedeemsOnlyOnce() async throws {
  let id = UUID()
  let account = UUID()
  let credentials = TestCredentials()
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(id)\",\"nonce\":\"server-nonce-with-at-least-forty-three-characters\",\"state\":\"placeholder\"}"
    )
  ])
  let auth = NativeAuthentication(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    credentials: credentials
  )
  // The server must echo the app state; an unrelated response cannot start authorization.
  await #expect(throws: AuthenticationError.self) { try await auth.beginApple() }
  #expect(await credentials.read(accountId: account) == nil)
}

actor AppleHTTP: HTTPTransport {
  var requests: [URLRequest] = []
  let account: UUID
  var reply: HTTPResponse?
  let nonce: String

  init(
    account: UUID = UUID(),
    reply: HTTPResponse? = nil,
    nonce: String = "server-nonce-with-at-least-forty-three-characters"
  ) {
    self.account = account
    self.reply = reply
    self.nonce = nonce
  }

  func send(_ request: URLRequest) async throws -> HTTPResponse {
    requests.append(request)
    if request.url?.path == "/auth/native/apple/attempts" {
      let body = try JSONSerialization.jsonObject(with: request.httpBody!) as! [String: String]
      return HTTPResponse(
        status: 201,
        body: try JSONSerialization.data(withJSONObject: [
          "attempt_id": UUID().uuidString, "state": body["state"]!,
          "nonce": nonce,
        ])
      )
    }
    if let reply { return reply }
    return response(
      200,
      "{\"session_token\":\"native-apple-session\",\"user\":{\"id\":\"\(account)\",\"display_name\":\"alice\"},\"identities\":[],\"idle_deadline\":\"2026-11-04T00:00:00Z\",\"absolute_deadline\":\"2027-01-01T00:00:00Z\"}"
    )
  }
}

@Test
func nativeAppleRejectsStateCodeAndReplayBeforeSendingCredentials() async throws {
  let transport = AppleHTTP()
  let credentials = TestCredentials()
  let auth = NativeAuthentication(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    credentials: credentials
  )
  let pending = try await auth.beginApple()
  for (code, state) in [
    ("code", "wrong"), ("", pending.state), (String(repeating: "a", count: 4097), pending.state),
  ] {
    await #expect(throws: AuthenticationError.self) {
      try await auth.completeApple(code: code, state: state, pending: pending)
    }
  }
  #expect(await transport.requests.count == 1)
  let result = try await auth.completeApple(
    code: "apple-code",
    state: pending.state,
    pending: pending
  )
  #expect(await credentials.read(accountId: result.user.id)?.token == "native-apple-session")
  await #expect(throws: AuthenticationError.self) {
    try await auth.completeApple(code: "apple-code", state: pending.state, pending: pending)
  }
  #expect(await transport.requests.count == 2)
  let body = try #require(
    JSONSerialization.jsonObject(with: await transport.requests[1].httpBody!) as? [String: String]
  )
  #expect(body["authorization_code"] == "apple-code")
  #expect(body["verifier"]?.count == 43)
  #expect(body["nonce"] == nil)
}

@Test
func nativeAppleLinkingAndDeletionNeverAcceptAnotherAccountOrReplaceDeletionSession() async throws {
  let account = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "existing"))
  let proof = response(
    200,
    "{\"proof_token\":\"purpose-bound\",\"account_id\":\"\(account)\",\"purpose\":\"account_deletion\",\"provider\":\"apple\",\"expires_at\":\"2026-11-04T00:00:00Z\"}"
  )
  let transport = AppleHTTP(reply: proof)
  let auth = NativeAuthentication(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    credentials: credentials
  )
  let pending = try await auth.beginApple(deletionAccountId: account)
  await #expect(throws: AuthenticationError.self) {
    try await auth.completeApple(code: "code", state: pending.state, pending: pending)
  }
  let result = try await auth.completeAppleReauthentication(
    code: "code",
    state: pending.state,
    pending: pending
  )
  #expect(result.proofToken == "purpose-bound")
  #expect(await credentials.read(accountId: account)?.token == "existing")
  let first = await transport.requests[0]
  #expect(first.value(forHTTPHeaderField: "Authorization") == "Bearer existing")
  let body = try #require(JSONSerialization.jsonObject(with: first.httpBody!) as? [String: String])
  #expect(body["mode"] == "reauth_delete")
  let linkHTTP = AppleHTTP()
  let linking = NativeAuthentication(
    client: APIClient(origin: try testOrigin(), transport: linkHTTP, credentials: credentials),
    credentials: credentials
  )
  let link = try await linking.beginApple(linkingAccountId: account)
  await #expect(throws: WorkspaceError.self) {
    try await linking.completeApple(code: "code", state: link.state, pending: link)
  }
  let canceled = try await linking.beginApple()
  await linking.cancel(canceled)
  await #expect(throws: AuthenticationError.self) {
    try await linking.completeApple(code: "code", state: canceled.state, pending: canceled)
  }
  await #expect(throws: AuthenticationError.self) {
    try await linking.beginApple(linkingAccountId: account, deletionAccountId: account)
  }
}

@Test
func nativeAppleRejectsMalformedNoncesAndDeletionProofs() async throws {
  let account = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "existing"))
  for nonce in [String(repeating: "a", count: 42), String(repeating: "a", count: 129)] {
    let transport = AppleHTTP(nonce: nonce)
    let auth = NativeAuthentication(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials
    )
    await #expect(throws: AuthenticationError.self) { try await auth.beginApple() }
  }
  for field in ["account_id", "provider", "purpose", "proof_token"] {
    var proof = [
      "account_id": account.uuidString, "provider": "apple", "purpose": "account_deletion",
      "proof_token": "proof", "expires_at": "2026-11-04T00:00:00Z",
    ]
    proof[field] = field == "account_id" ? UUID().uuidString : ""
    let transport = AppleHTTP(
      reply: HTTPResponse(status: 200, body: try JSONSerialization.data(withJSONObject: proof))
    )
    let auth = NativeAuthentication(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      credentials: credentials
    )
    let pending = try await auth.beginApple(deletionAccountId: account)
    await #expect(throws: WorkspaceError.self) {
      try await auth.completeAppleReauthentication(
        code: "code",
        state: pending.state,
        pending: pending
      )
    }
    let signIn = try await auth.beginApple()
    await #expect(throws: AuthenticationError.self) {
      try await auth.completeAppleReauthentication(
        code: "code",
        state: signIn.state,
        pending: signIn
      )
    }
  }
  #expect(await credentials.read(accountId: account)?.token == "existing")
}
