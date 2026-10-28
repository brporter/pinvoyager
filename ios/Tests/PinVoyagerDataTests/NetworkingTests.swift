import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func authenticationCapabilityDoesNotPretendSynchronizationIsAvailable() throws {
  let body = Data(
    "{\"native_authentication\":true,\"native_protocol_versions\":[],\"providers\":[\"google\"],\"formats\":[\"jpeg\"],\"max_upload_bytes\":26214400,\"max_order_entries\":10000,\"max_manifest_entries\":10000,\"owner_offline_downloads\":false}"
      .utf8)
  let capabilities = try APIJSON.decoder().decode(ClientCapabilities.self, from: body)
  #expect(capabilities.supportsNativeAuthentication)
  #expect(!capabilities.supportsNativeSync)
}

actor StubHTTP: HTTPTransport {
  var requests: [URLRequest] = []
  var replies: [HTTPResponse]
  init(_ replies: [HTTPResponse]) { self.replies = replies }
  func send(_ request: URLRequest) async throws -> HTTPResponse {
    requests.append(request)
    guard !replies.isEmpty else { throw URLError(.notConnectedToInternet) }
    return replies.removeFirst()
  }
}

actor TestCredentials: CredentialStore {
  var records: [UUID: SessionCredential] = [:]
  func read(accountId: UUID) -> SessionCredential? { records[accountId] }
  func save(_ credential: SessionCredential) { records[credential.accountId] = credential }
  func remove(accountId: UUID) { records.removeValue(forKey: accountId) }
  func all() -> [SessionCredential] { Array(records.values) }
}

func response(_ status: Int, _ json: String, headers: [String: String] = [:]) -> HTTPResponse {
  HTTPResponse(status: status, headers: headers, body: Data(json.utf8))
}

func testOrigin() throws -> TrustedOrigin {
  try TrustedOrigin(URL(string: "https://pinvoyager.com")!)
}

@Test func readsActualRustFixtures() async throws {
  let fixtures = Fixtures.directory("api")
  for name in ["voyage_detail", "voyage_created", "voyage_updated"] {
    let voyage = try APIJSON.decoder().decode(
      RemoteVoyage.self, from: Data(contentsOf: fixtures.appendingPathComponent("\(name).json")))
    #expect(!voyage.title.isEmpty)
    #expect(voyage.photos.count == voyage.photoCount)
  }
  let page = try APIJSON.decoder().decode(
    VoyagePage.self, from: Data(contentsOf: fixtures.appendingPathComponent("voyage_list.json")))
  #expect(!page.voyages.isEmpty)
  let me = try APIJSON.decoder().decode(
    AccountResponse.self, from: Data(contentsOf: fixtures.appendingPathComponent("me.json")))
  #expect(me.user.displayName == "fixture-owner")
  let map = try APIJSON.decoder().decode(
    DiscoveryResponse.self,
    from: Data(contentsOf: fixtures.appendingPathComponent("discover_map.json")))
  #expect(!map.clusters.isEmpty)
}

@Test func credentialsStayOnTrustedOriginAndNeverUseCookies() async throws {
  let id = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: id, token: "secret"))
  let transport = StubHTTP([response(200, "{\"voyages\":[],\"next_before\":null}")])
  let client = APIClient(origin: try testOrigin(), transport: transport, credentials: credentials)
  let _: VoyagePage = try await client.get(path: "/api/voyages", accountId: id)
  let request = await transport.requests.first!
  #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer secret")
  #expect(!request.httpShouldHandleCookies)
  #expect(request.cachePolicy == .reloadIgnoringLocalCacheData)
  await #expect(throws: OriginError.self) {
    let _: VoyagePage = try await client.get(path: "https://evil.test/api/voyages", accountId: id)
  }
  #expect(await transport.requests.count == 1)
}

@Test func unsupportedServerIsExplicitAndPublicBrowsingStillWorks() async throws {
  let transport = StubHTTP([
    response(404, ""), response(200, "{\"voyages\":[],\"next_before\":null}"),
  ])
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: TestCredentials())
  #expect(try await client.capabilities() == nil)
  #expect(try await client.voyages().voyages.isEmpty)
}

@Test func errorParsingIsSafeAndPreservesRequestReferences() async throws {
  let transport = StubHTTP([
    response(
      412, "{\"code\":\"conflict\",\"message\":\"Changed\"}",
      headers: ["x-request-id": "ref", "retry-after": "120"]),
    response(503, "<html>proxy private detail</html>"),
  ])
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: TestCredentials())
  do {
    let _: VoyagePage = try await client.get(path: "/api/voyages")
    Issue.record("Expected 412")
  } catch let error as APIError {
    #expect(error.status == 412)
    #expect(error.requestId == "ref")
    #expect(error.retryAfter == 120)
  }
  do {
    let _: VoyagePage = try await client.get(path: "/api/voyages")
    Issue.record("Expected 503")
  } catch let error as APIError { #expect(!error.message.contains("private detail")) }
}

@Test func malformedSuccessDoesNotCountAsAcceptance() async throws {
  let transport = StubHTTP([response(201, "<html>login</html>")])
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: TestCredentials())
  await #expect(throws: DecodingError.self) {
    let _: RemoteVoyage = try await client.get(path: "/api/voyages")
  }
}

@Test func authenticationValidatesStateAndOriginBeforeExchangingCode() async throws {
  let attempt = UUID()
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(attempt)\",\"browser_start_url\":\"https://pinvoyager.com/auth/native/attempts/\(attempt)/start\"}"
    )
  ])
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: TestCredentials())
  let auth = NativeAuthentication(client: client, credentials: TestCredentials())
  let pending = try await auth.begin(provider: "apple")
  #expect(pending.verifier.count >= 43)
  #expect(pending.state.count >= 43)
  await #expect(throws: AuthenticationError.self) {
    try await auth.complete(
      URL(string: "pinvoyager://auth/callback?code=secret&state=wrong")!, pending: pending)
  }
  await #expect(throws: AuthenticationError.self) {
    try await auth.complete(
      URL(string: "evil://auth/callback?code=secret&state=\(pending.state)")!, pending: pending)
  }
  #expect(await transport.requests.count == 1)
}

@Test func retryAfterSupportsHTTPDate() {
  let now = Date(timeIntervalSince1970: 0)
  #expect(APIError.retryDelay("Thu, 01 Jan 1970 00:01:00 GMT", now: now) == 60)
  #expect(APIError.retryDelay("bad", now: now) == nil)
}

@Test func nativeContractsMatchRustCapturedFixtures() async throws {
  let directory = Fixtures.directory("api")
  func fixture(_ name: String) throws -> Data {
    try Data(contentsOf: directory.appendingPathComponent("\(name).json"))
  }
  let capabilities = try APIJSON.decoder().decode(
    ClientCapabilities.self, from: fixture("native_capabilities"))
  #expect(capabilities.supportsNativeSync)
  #expect(capabilities.ownerOfflineDownloads)
  let session = try APIJSON.decoder().decode(
    SessionExchange.self, from: fixture("native_session_exchange"))
  #expect(session.user.displayName == nil)
  #expect(!session.sessionToken.isEmpty)
  #expect(APIJSON.date(session.absoluteDeadline) != nil)
  let manifest = try APIJSON.decoder().decode(
    OfflineManifest.self, from: fixture("native_offline_manifest"))
  #expect(manifest.retentionPermission == "owner_offline_copy_v1")
  #expect(manifest.photos.first?.variants.count == 3)
  struct Digest: Decodable {
    let title: String
    let description: String
    let granularity: Granularity
    let canonicalUtf8: String
    let sha256: String
  }
  let digest = try APIJSON.decoder().decode(Digest.self, from: fixture("native_metadata_digest"))
  #expect(
    try RequestFactory.metadataDigest(
      VoyageFields(
        title: digest.title, description: digest.description, granularity: digest.granularity))
      == digest.sha256)
  let credentials = TestCredentials()
  let account = UUID()
  await credentials.save(SessionCredential(accountId: account, token: "session"))
  let remote = NativeSyncRemote(
    client: APIClient(
      origin: try testOrigin(),
      transport: StubHTTP([
        HTTPResponse(status: 200, body: try fixture("native_operation_applied")),
        HTTPResponse(status: 200, body: try fixture("native_publication_published")),
      ]), credentials: credentials), accountId: account)
  if case .applied(_, let revision, _) = try await remote.receipt(operationId: UUID()) {
    #expect(revision == "1")
  } else {
    Issue.record("Expected a captured applied receipt")
  }
  if case .publication(_, .published, _, let confirmed, _) = try await remote.publicationStatus(
    voyageId: UUID(), intentId: UUID())
  {
    #expect(confirmed)
  } else {
    Issue.record("Expected current publication status")
  }
}

private actor DelayedImageTransport: HTTPTransport {
  var requestCount = 0
  var active = 0
  var peak = 0
  func send(_ request: URLRequest) async throws -> HTTPResponse {
    requestCount += 1
    active += 1
    peak = max(peak, active)
    defer { active -= 1 }
    try await Task.sleep(for: .milliseconds(30))
    return HTTPResponse(status: 200, body: Data([1, 2, 3]))
  }
}

@Test func simultaneousImagesShareOneRequestAndEveryLaterReadRevalidates() async throws {
  let transport = DelayedImageTransport()
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: TestCredentials())
  let path = "/api/photos/\(UUID())/thumb"
  async let first = client.image(path)
  async let second = client.image(path)
  #expect(try await first == second)
  #expect(await transport.requestCount == 1)
  _ = try await client.image(path)
  #expect(await transport.requestCount == 2)
}

@Test func imageRequestsAreBoundedAndNeverSharedBetweenAccounts() async throws {
  let transport = DelayedImageTransport()
  let credentials = TestCredentials()
  let first = UUID()
  let second = UUID()
  await credentials.save(SessionCredential(accountId: first, token: "first"))
  await credentials.save(SessionCredential(accountId: second, token: "second"))
  let client = APIClient(origin: try testOrigin(), transport: transport, credentials: credentials)
  let path = "/api/photos/\(UUID())/thumb"
  async let one = client.image(path, accountId: first)
  async let two = client.image(path, accountId: second)
  _ = try await (one, two)
  #expect(await transport.requestCount == 2)
  try await withThrowingTaskGroup(of: Data.self) { group in
    for _ in 0..<12 { group.addTask { try await client.image("/api/photos/\(UUID())/thumb") } }
    for try await _ in group {}
  }
  #expect(await transport.peak <= 4)
}

@Test func rustCapturedRejectionStatusReachesNativeConflictClassification() async throws {
  let body = try Data(
    contentsOf: Fixtures.directory("api").appendingPathComponent("native_operation_rejected.json"))
  let account = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "session"))
  let remote = NativeSyncRemote(
    client: APIClient(
      origin: try testOrigin(), transport: StubHTTP([HTTPResponse(status: 200, body: body)]),
      credentials: credentials), accountId: account)
  if case .rejected(let message, let status) = try await remote.receipt(operationId: UUID()) {
    #expect(status == 412)
    #expect(!message.isEmpty)
  } else {
    Issue.record("The native client must retain the rejection's original status")
  }
}

private actor DelayedProfileTransport: HTTPTransport {
  var requests: [URLRequest] = []
  let body: Data
  init(body: Data) { self.body = body }
  func send(_ request: URLRequest) async throws -> HTTPResponse {
    requests.append(request)
    try await Task.sleep(for: .milliseconds(30))
    return HTTPResponse(status: 200, body: body)
  }
}

@Test func profileNamesShareConcurrentRequestsAndKnownAliasesUseTheSameAccountIdentity()
  async throws
{
  let id = UUID()
  let body = Data(
    "{\"user\":{\"id\":\"\(id)\",\"display_name\":\"current-name\"},\"voyages\":[],\"next_before\":null}"
      .utf8)
  let transport = DelayedProfileTransport(body: body)
  let credentials = TestCredentials()
  let client = APIClient(origin: try testOrigin(), transport: transport, credentials: credentials)
  async let first = client.profile("Old-Name")
  async let second = client.profile("old-name")
  #expect(try await first.user.id == second.user.id)
  #expect(await transport.requests.count == 1)
  async let old = client.profile("old-name")
  async let current = client.profile("current-name")
  async let canonical = client.profile(id.uuidString)
  _ = try await (old, current, canonical)
  #expect(await transport.requests.count == 2)
  #expect(
    await transport.requests.last?.url?.path.lowercased()
      == "/api/profiles/\(id.uuidString.lowercased())")
  let account = UUID()
  await credentials.save(SessionCredential(accountId: account, token: "viewer"))
  async let signed = client.profile("old-name", accountId: account)
  async let guest = client.profile("old-name")
  _ = try await (signed, guest)
  #expect(await transport.requests.count == 4)
  #expect(
    await transport.requests.contains {
      $0.value(forHTTPHeaderField: "Authorization") == "Bearer viewer"
    })
}

@Test func canonicalProfilesRejectAnotherTravelersIdentity() async throws {
  let id = UUID()
  let other = UUID()
  let body = Data(
    "{\"user\":{\"id\":\"\(other)\",\"display_name\":\"other\"},\"voyages\":[],\"next_before\":null}"
      .utf8)
  let client = APIClient(
    origin: try testOrigin(), transport: StubHTTP([HTTPResponse(status: 200, body: body)]),
    credentials: TestCredentials())
  await #expect(throws: WorkspaceError.malformedReceipt) { try await client.profile(id.uuidString) }
}
