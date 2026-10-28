import CryptoKit
import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

func downloadManifest(
  account: UUID, voyage: UUID, photo: UUID, bytes: Data, revision: String = "3"
) throws -> Data {
  let hash = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
  return try JSONSerialization.data(withJSONObject: [
    "voyage_id": voyage.uuidString, "owner_id": account.uuidString, "revision": revision,
    "title": "Winter", "description": "On foot", "granularity": "km1", "published": false,
    "retention_permission": "owner_offline_copy_v1", "authorized_at": "2026-10-25T10:00:00Z",
    "photos": [
      [
        "id": photo.uuidString, "position": 1, "width": 2, "height": 2,
        "variants": ["thumb", "display", "full"].map { kind in
          [
            "kind": kind, "url": "/api/photos/\(photo)/offline/\(kind)", "byte_count": bytes.count,
            "sha256": hash,
          ] as [String: Any]
        },
      ]
    ],
  ])
}

@Test func ownerDownloadCommitsOnlyVerifiedFilesAndReopensOffline() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let store = try CoreDataWorkspaceStore(url: directory.appendingPathComponent("workspace.sqlite"))
  let account = UUID()
  let remote = UUID()
  let photo = UUID()
  var voyage = LocalVoyage(
    ownerId: account, fields: VoyageFields(title: "Winter", description: "On foot"))
  voyage.serverId = remote
  var workspace = Workspace(account: LocalAccount(profile: UserProfile(id: account)))
  workspace.voyages = [voyage]
  try await store.insert(workspace)
  let bytes = Data("RIFF1234WEBPstripped".utf8)
  let manifest = try downloadManifest(account: account, voyage: remote, photo: photo, bytes: bytes)
  let transport = StubHTTP(
    [HTTPResponse(status: 200, body: manifest)]
      + (0..<2).map { _ in
        HTTPResponse(
          status: 200, headers: ["x-pinvoyager-retention": "owner_offline_copy_v1"], body: bytes)
      } + [HTTPResponse(status: 200, body: manifest)])
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let vault = try FileVault(root: directory.appendingPathComponent("files"))
  let service = OwnerDownloads(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    store: store, vault: vault)
  try await service.download(accountId: account, voyageId: voyage.id)
  let reopened = try CoreDataWorkspaceStore(
    url: directory.appendingPathComponent("workspace.sqlite"))
  let saved = try await reopened.read(accountId: account)
  let local = try #require(saved.photos.first)
  #expect(local.source == nil)
  #expect(local.offlineCopy?.variants["full"] == nil)
  #expect(local.offlineCopy?.revision == "3")
  #expect(
    try await vault.read(try #require(local.offlineCopy?.variants["display"]), accountId: account)
      == bytes)
  try await service.remove(accountId: account, voyageId: voyage.id)
  #expect(try await store.read(accountId: account).photos.first?.offlineCopy == nil)
}

@Test func damagedDownloadsAndChangedSnapshotsNeverBecomeOfflineCopies() async throws {
  for changed in [false, true] {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let account = UUID()
    let remote = UUID()
    let photo = UUID()
    var voyage = LocalVoyage(
      ownerId: account, fields: VoyageFields(title: "Winter", description: "On foot"))
    voyage.serverId = remote
    var workspace = Workspace(account: LocalAccount(profile: UserProfile(id: account)))
    workspace.voyages = [voyage]
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(workspace)
    let bytes = Data("RIFF1234WEBPstripped".utf8)
    let manifest = try downloadManifest(
      account: account, voyage: remote, photo: photo, bytes: bytes)
    let later = try downloadManifest(
      account: account, voyage: remote, photo: photo, bytes: bytes, revision: "4")
    let transport = StubHTTP(
      [HTTPResponse(status: 200, body: manifest)]
        + (0..<2).map { _ in
          HTTPResponse(
            status: 200, headers: ["x-pinvoyager-retention": "owner_offline_copy_v1"],
            body: changed ? bytes : Data("corrupt".utf8))
        } + [HTTPResponse(status: 200, body: later)])
    let credentials = TestCredentials()
    await credentials.save(SessionCredential(accountId: account, token: "token"))
    let service = OwnerDownloads(
      client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
      store: store, vault: try FileVault(root: directory))
    await #expect(throws: WorkspaceError.self) {
      try await service.download(accountId: account, voyageId: voyage.id)
    }
    #expect(try await store.read(accountId: account).photos.isEmpty)
  }
}

@Test func fullOwnerVariantsDownloadOnlyAfterAnExplicitRequest() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let state = workspaceWithPhotos(remote: true)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let account = state.account.id
  let remote = state.voyages[0].serverId!
  let remotePhoto = state.photos[0].serverId!
  let bytes = Data("RIFF1234WEBPstripped".utf8)
  let manifest = try downloadManifest(
    account: account, voyage: remote, photo: remotePhoto, bytes: bytes)
  let file = HTTPResponse(
    status: 200, headers: ["x-pinvoyager-retention": "owner_offline_copy_v1"], body: bytes)
  let metadata = HTTPResponse(status: 200, body: manifest)
  let transport = StubHTTP([metadata, file, file, metadata, metadata, file, metadata])
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  let service = OwnerDownloads(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    store: store, vault: try FileVault(root: directory))
  try await service.download(accountId: account, voyageId: state.voyages[0].id)
  #expect(
    try await store.read(accountId: account).photos.first { $0.id == state.photos[0].id }?
      .offlineCopy?.variants["full"] == nil)
  try await service.downloadFull(accountId: account, photoId: state.photos[0].id)
  #expect(
    try await store.read(accountId: account).photos.first { $0.id == state.photos[0].id }?
      .offlineCopy?.variants["full"] != nil)
  #expect(await transport.requests.filter { $0.url?.path.hasSuffix("/full") == true }.count == 1)
}

@Test func confirmedWebsiteRemovalDiscardsDownloadedGrantsAndKeepsImportedOriginals() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  var state = workspaceWithPhotos(remote: true)
  let original = state.photos[0]
  state.photos[1].source = nil
  for index in state.photos.indices {
    state.photos[index].offlineCopy = OfflinePhotoCopy(
      revision: "7", variants: ["thumb": "downloads/\(index).webp"], byteCount: 3)
  }
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let vault = try FileVault(root: directory)
  for index in state.photos.indices {
    try await vault.write(
      Data([1, 2, 3]), relativePath: "downloads/\(index).webp", accountId: state.account.id)
  }
  let service = OwnerDownloads(
    client: APIClient(
      origin: try testOrigin(), transport: StubHTTP([]), credentials: TestCredentials()),
    store: store, vault: vault)
  try await service.purgeMissing(
    accountId: state.account.id, voyageId: state.voyages[0].id, keeping: [],
    revision: state.voyages[0].revision!)
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.photos.allSatisfy { $0.offlineCopy == nil })
  #expect(saved.photos.first { $0.id == original.id }?.source == original.source)
  #expect(saved.photos.first { $0.id == original.id }?.deleted == false)
  #expect(saved.photos.first { $0.id == original.id }?.transfer == .needsAttention)
  #expect(saved.photos.first { $0.id == state.photos[1].id }?.deleted == true)
  await #expect(throws: (any Error).self) {
    try await vault.read("downloads/0.webp", accountId: state.account.id)
  }
}

@Test func localPendingChangesPreventOwnerDownloadAndProtectExistingGrantsFromPurge() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  var state = workspaceWithPhotos(remote: true)
  state.operations = [
    OutboxOperation(
      accountId: state.account.id, generation: 0,
      voyageId: state.voyages[0].id, kind: .edit(state.voyages[0].fields))
  ]
  state.photos[0].offlineCopy = OfflinePhotoCopy(
    revision: "7", variants: ["display": "downloads/kept.webp"], byteCount: 3)
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let vault = try FileVault(root: directory)
  try await vault.write(
    Data([1, 2, 3]), relativePath: "downloads/kept.webp", accountId: state.account.id)
  let transport = StubHTTP([])
  let service = OwnerDownloads(
    client: APIClient(
      origin: try testOrigin(), transport: transport, credentials: TestCredentials()), store: store,
    vault: vault)
  await #expect(throws: ValidationError.self) {
    try await service.download(accountId: state.account.id, voyageId: state.voyages[0].id)
  }
  try await service.purgeMissing(
    accountId: state.account.id, voyageId: state.voyages[0].id, keeping: [], revision: "7")
  #expect(
    try await store.read(accountId: state.account.id).photos.first { $0.id == state.photos[0].id }?
      .offlineCopy != nil)
  #expect(
    try await vault.read("downloads/kept.webp", accountId: state.account.id) == Data([1, 2, 3]))
  #expect(await transport.requests.isEmpty)
}
