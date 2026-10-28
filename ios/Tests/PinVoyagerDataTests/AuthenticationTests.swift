import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func completedAuthenticationIsSingleUseAndStoresOnlyAppSession() async throws {
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
    string: "pinvoyager://auth/callback?code=single-use-code&state=\(pending.state)")!
  let reply = try await authentication.complete(callback, pending: pending)
  #expect(reply.user.id == accountId)
  #expect(await credentials.read(accountId: accountId)?.token == "opaque-app-session")
  await #expect(throws: AuthenticationError.self) {
    try await authentication.complete(callback, pending: pending)
  }
  #expect(await transport.requests.count == 2)
  let body = try #require(
    JSONSerialization.jsonObject(with: await transport.requests[0].httpBody!) as? [String: String])
  #expect(body["mode"] == "sign_in")
  #expect(body["challenge"]?.count == 43)
  #expect(body["verifier"] == nil)
}

@Test func revocationOnlyCredentialsCannotUploadAndCanRevokeThemselves() async throws {
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
      == "Bearer old-session")
}

@Test func authenticationRefusesForeignStartURLsAndUnsupportedProviders() async throws {
  let transport = StubHTTP([
    response(
      201, "{\"attempt_id\":\"\(UUID())\",\"browser_start_url\":\"https://evil.test/start\"}")
  ])
  let auth = NativeAuthentication(
    client: APIClient(
      origin: try testOrigin(), transport: transport, credentials: TestCredentials()),
    credentials: TestCredentials())
  await #expect(throws: AuthenticationError.self) { try await auth.begin(provider: "password") }
  await #expect(throws: AuthenticationError.self) { try await auth.begin(provider: "google") }
}

@Test func authenticationAcceptsCanonicalLowercaseServerUUIDs() async throws {
  let id = UUID()
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(id.uuidString.lowercased())\",\"browser_start_url\":\"https://pinvoyager.com/auth/native/attempts/\(id.uuidString.lowercased())/start\"}"
    )
  ])
  let auth = NativeAuthentication(
    client: APIClient(
      origin: try testOrigin(), transport: transport, credentials: TestCredentials()),
    credentials: TestCredentials())
  let pending = try await auth.begin(provider: "google")
  #expect(pending.id == id)
  await auth.cancel(pending)
}

#if os(iOS)
  @Test func keychainRoundTripAndRemovalUseAnIsolatedService() async throws {
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
