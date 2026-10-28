import CryptoKit
import Foundation
import ImageIO
import PinVoyagerDomain
import Testing
import UniformTypeIdentifiers

@testable import PinVoyagerData

@Test func nativeRemoteRequiresDurableReceiptsAndStrongRevisionHeaders() async throws {
  let accountId = UUID()
  let operationId = UUID()
  let targetId = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: accountId, token: "token"))
  let caps =
    "{\"native_protocol_versions\":[1],\"providers\":[\"apple\"],\"formats\":[\"jpeg\"],\"max_upload_bytes\":26214400,\"max_order_entries\":10000,\"max_manifest_entries\":10000,\"owner_offline_downloads\":false}"
  let transport = StubHTTP([
    response(200, caps), response(204, ""),
    response(200, "{\"status\":\"applied\",\"target_id\":\"\(targetId)\",\"revision\":\"9\"}"),
  ])
  let remote = NativeSyncRemote(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    accountId: accountId)
  #expect(try await remote.supportsNativeSync())
  let request = MutationRequest(
    method: "PATCH", path: "/api/voyages/\(targetId)", body: Data("{}".utf8), revision: "8")
  #expect(
    try await remote.submit(request, operationId: operationId)
      == .applied(targetId: targetId, revision: "9"))
  let submitted = await transport.requests[1]
  #expect(submitted.value(forHTTPHeaderField: "If-Match") == "\"v:8\"")
  #expect(submitted.value(forHTTPHeaderField: "Idempotency-Key") == operationId.uuidString)
  #expect(await transport.requests[2].url?.path == "/api/sync/operations/\(operationId)")
}

@Test func remoteReceiptStatesAndMalformedResponsesAreDeliberate() async throws {
  let account = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let id = UUID()
  for (status, expected) in [
    ("not_seen", OperationReceipt.notSeen), ("in_progress", .inProgress), ("deleted", .deleted),
    ("canceled", .deleted), ("rejected", .rejected("This change needs attention.")),
  ] {
    let remote = NativeSyncRemote(
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([response(200, "{\"status\":\"\(status)\"}")]), credentials: credentials
      ), accountId: account)
    #expect(try await remote.receipt(operationId: id) == expected)
  }
  for status in ["applied", "accepted", "future"] {
    let remote = NativeSyncRemote(
      client: APIClient(
        origin: try testOrigin(),
        transport: StubHTTP([response(200, "{\"status\":\"\(status)\"}")]), credentials: credentials
      ), accountId: account)
    await #expect(throws: WorkspaceError.self) { try await remote.receipt(operationId: id) }
  }
  let transport = StubHTTP([
    response(200, "{\"status\":\"accepted\",\"photo_id\":\"\(id)\",\"revision\":\"4\"}"),
    response(
      200, "{\"id\":\"\(id)\",\"status\":\"processing\",\"revision\":\"5\",\"published\":false}"),
    response(
      200,
      "{\"status\":\"applied\",\"publication_intent_id\":\"\(id)\",\"publication_status\":\"processing\",\"revision\":\"6\"}"
    ),
  ])
  let remote = NativeSyncRemote(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    accountId: account)
  #expect(
    try await remote.uploadReceipt(voyageId: UUID(), key: UUID())
      == .applied(targetId: id, revision: "4"))
  #expect(
    try await remote.publicationStatus(voyageId: UUID(), intentId: id)
      == .publication(intentId: id, status: .processing, revision: "5"))
  #expect(
    try await remote.receipt(operationId: UUID())
      == .publication(intentId: id, status: .processing, revision: "6"))
}

@Test func imageAndProfileRequestsCannotEscapeExpectedRoutes() async throws {
  let transport = StubHTTP([response(200, "image-bytes")])
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: TestCredentials())
  let id = UUID()
  #expect(try await client.image("/api/photos/\(id)/thumb") == Data("image-bytes".utf8))
  for path in [
    "/originals/\(id)", "/api/photos/\(id)/original", "https://evil.test/api/photos/\(id)/full",
    "/api/photos/no/thumb",
  ] {
    await #expect(throws: OriginError.self) { try await client.image(path) }
  }
  await #expect(throws: OriginError.self) { try await client.profile("../me") }
  let page = try Data(
    contentsOf: Fixtures.directory("api").appendingPathComponent("public_profile.json"))
  let response = StubHTTP([HTTPResponse(status: 200, body: page)])
  let profile = try await APIClient(
    origin: testOrigin(), transport: response, credentials: TestCredentials()
  ).profile("alice", before: "opaque|cursor")
  #expect(profile.user.displayName != nil)
  #expect(await response.requests.first?.url?.query?.contains("before=") == true)
}

@Test func fileFormatsExportAndAccountRemovalPreserveUnsyncedContentUntilExplicitAction()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let vault = try FileVault(root: directory.appendingPathComponent("vault"))
  let account = UUID()
  let inputs: [(Data, String)] = [
    (Data([137, 80, 78, 71, 13, 10, 26, 10, 0]), "image/png"),
    (Data("RIFF1234WEBPtail".utf8), "image/webp"),
    (Data([0, 0, 0, 20] + Array("ftypheicmore".utf8)), "image/heic"),
  ]
  for (index, entry) in inputs.enumerated() {
    let input = directory.appendingPathComponent("source\(index)")
    try entry.0.write(to: input)
    let file = try await vault.importFile(input, accountId: account, photoId: UUID(), maxBytes: 100)
    #expect(file.mediaType == entry.1)
    #expect(
      file.contentHash == SHA256.hash(data: entry.0).map { String(format: "%02x", $0) }.joined())
  }
  await #expect(throws: FileVaultError.self) {
    try await vault.importFile(
      directory.appendingPathComponent("missing"), accountId: account, photoId: UUID(),
      maxBytes: 100)
  }
  let empty = directory.appendingPathComponent("empty")
  try Data().write(to: empty)
  await #expect(throws: FileVaultError.self) {
    try await vault.importFile(empty, accountId: account, photoId: UUID(), maxBytes: 100)
  }
  var state = workspaceWithPhotos()
  state.photos = [state.photos[0]]
  try await vault.write(
    Data("private source".utf8), relativePath: state.photos[0].source!.relativePath,
    accountId: state.account.id)
  let export = try await vault.export(
    state, destination: directory.appendingPathComponent("export"))
  #expect(
    FileManager.default.fileExists(atPath: export.appendingPathComponent("workspace.json").path))
  #expect(
    try Data(contentsOf: export.appendingPathComponent(state.photos[0].id.uuidString))
      == Data("private source".utf8))
  try await vault.removeAccount(state.account.id)
  try await vault.removeAccount(state.account.id)
  await #expect(throws: (any Error).self) {
    try await vault.read(state.photos[0].source!.relativePath, accountId: state.account.id)
  }
}

@Test func exifLocationAndDateRemainPrivateAndPreviewStripsMetadata() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let fixture = Fixtures.directory("images").appendingPathComponent("quadrants-rgb.heic")
  let source = try #require(CGImageSourceCreateWithURL(fixture as CFURL, nil))
  let image = try #require(CGImageSourceCreateImageAtIndex(source, 0, nil))
  let url = directory.appendingPathComponent("located.jpg")
  let destination = try #require(
    CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil))
  let metadata: [CFString: Any] = [
    kCGImagePropertyGPSDictionary: [
      kCGImagePropertyGPSLatitude: 33.5, kCGImagePropertyGPSLatitudeRef: "S",
      kCGImagePropertyGPSLongitude: 151.25, kCGImagePropertyGPSLongitudeRef: "E",
    ],
    kCGImagePropertyExifDictionary: [kCGImagePropertyExifDateTimeOriginal: "2025:02:14 09:31:07"],
  ]
  CGImageDestinationAddImage(destination, image, metadata as CFDictionary)
  #expect(CGImageDestinationFinalize(destination))
  let vault = try FileVault(root: directory.appendingPathComponent("vault"))
  let account = UUID()
  let photo = try await PhotoPreparer(vault: vault).prepare(
    url, accountId: account, voyageId: UUID(), maxBytes: 1_000_000)
  #expect(photo.coordinate == Coordinate(latitude: -33.5, longitude: 151.25))
  #expect(photo.capturedAt == "2025:02:14 09:31:07")
  let preview = try await vault.read(photo.previewPath!, accountId: account)
  let previewSource = try #require(CGImageSourceCreateWithData(preview as CFData, nil))
  let properties = CGImageSourceCopyPropertiesAtIndex(previewSource, 0, nil) as? [CFString: Any]
  #expect(properties?[kCGImagePropertyGPSDictionary] == nil)
}

@Test func storeRejectsCrossAccountWritesAndPreservesCorruptFiles() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = unlockedWorkspace()
  try await store.insert(state)
  #expect(try await store.accountIds() == [state.account.id])
  await #expect(throws: WorkspaceError.self) { try await store.insert(state) }
  await #expect(throws: WorkspaceError.self) { try await store.read(accountId: UUID()) }
  await #expect(throws: WorkspaceError.self) {
    try await store.update(accountId: state.account.id) { state in
      state.voyages.append(
        LocalVoyage(ownerId: UUID(), fields: VoyageFields(title: "Other", description: "Account")))
    }
  }
  #expect(try await store.read(accountId: state.account.id).voyages.isEmpty)
  try await store.remove(accountId: state.account.id)
  #expect(try await store.accountIds().isEmpty)
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("corrupt.sqlite")
  let bytes = Data("recover me".utf8)
  try bytes.write(to: url)
  #expect(throws: (any Error).self) { try CoreDataWorkspaceStore(url: url) }
  #expect(try Data(contentsOf: url) == bytes)
}

@Test func aHistoricalPublicationReceiptRevalidatesCurrentVisibility() async throws {
  let account = UUID()
  let voyage = UUID()
  let intent = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let transport = StubHTTP([
    response(
      200,
      "{\"status\":\"applied\",\"voyage_id\":\"\(voyage)\",\"publication_intent_id\":\"\(intent)\",\"publication_status\":\"published\",\"revision\":\"6\",\"previous_revision\":\"5\"}"
    ),
    response(200, "{\"published\":false,\"revision\":\"9\"}"),
  ])
  let remote = NativeSyncRemote(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    accountId: account)
  #expect(
    try await remote.receipt(operationId: UUID())
      == .publication(
        intentId: intent, status: .published, revision: "6", confirmedPublished: false,
        previousRevision: "5"))
  #expect(await transport.requests.last?.url?.path == "/api/voyages/\(voyage)")
}
