import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

func conflictSnapshot(
  account: UUID, voyage: UUID, revision: String = "12", ids: [UUID], published: Bool = false
)
  -> OwnerVoyageSnapshot
{
  OwnerVoyageSnapshot(
    voyage: RemoteVoyage(
      id: voyage, ownerId: account, title: "Website name",
      description: "Website description", published: published, granularity: .km1,
      granularityLabel: "1 km",
      photoCount: ids.count, placedCount: 0, placeCount: 0, unplacedCount: ids.count,
      photos: ids.enumerated().map { position, id in
        RemotePhoto(
          id: id, position: position, capturedAt: nil,
          width: 2, height: 2, variantStatus: .ready, thumbnailUrl: "/thumb",
          displayUrl: "/display", fullUrl: "/full",
          coordinate: nil, placed: false)
      }, revision: revision),
    photos: ids.map {
      OwnerPhoto(
        id: $0, width: 2, height: 2, capturedAt: nil, latitude: 10, longitude: 20,
        variantStatus: .ready)
    })
}

@Test func ownerSnapshotRejectsChangesDuringPaginationAndUsesOnlyPrivateMetadata() async throws {
  let account = UUID()
  let voyage = UUID()
  let first = UUID()
  let second = UUID()
  let snapshot = conflictSnapshot(account: account, voyage: voyage, ids: [first, second])
  let before = try APIJSON.encode(snapshot.voyage)
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "token"))
  for changed in [false, true] {
    let after = try APIJSON.encode(
      conflictSnapshot(
        account: account, voyage: voyage, revision: changed ? "13" : "12", ids: [first, second]
      ).voyage)
    let transport = StubHTTP([
      HTTPResponse(status: 200, body: before),
      response(
        200,
        "{\"photos\":[{\"id\":\"\(first)\",\"width\":2,\"height\":2,\"latitude\":10.123,\"longitude\":20.456,\"variant_status\":\"ready\"}],\"next_before\":\"second-page\"}"
      ),
      response(
        200,
        "{\"photos\":[{\"id\":\"\(second)\",\"width\":2,\"height\":2,\"variant_status\":\"ready\"}],\"next_before\":null}"
      ),
      HTTPResponse(status: 200, body: after),
    ])
    let client = APIClient(origin: try testOrigin(), transport: transport, credentials: credentials)
    if changed {
      await #expect(throws: ValidationError.self) {
        try await client.ownerSnapshot(voyageId: voyage, accountId: account)
      }
    } else {
      let result = try await client.ownerSnapshot(voyageId: voyage, accountId: account)
      #expect(result.photos.first?.coordinate == Coordinate(latitude: 10.123, longitude: 20.456))
      #expect(result.photos.last?.coordinate == nil)
      #expect(await transport.requests[2].url?.query == "before=second-page")
    }
  }
}

@Test func combinedPhotoResolutionRequiresEveryReviewedItemAndWaitsForUncertainWrites() async throws
{
  var initial = workspaceWithPhotos(remote: true)
  let voyage = initial.voyages[0]
  var conflict = OutboxOperation(
    accountId: initial.account.id, generation: 0, voyageId: voyage.id,
    kind: .order(initial.photos.map(\.id)), expectedRevision: "7")
  conflict.state = .conflict
  initial.operations = [conflict]
  let ids = initial.photos.compactMap(\.serverId)
  let snapshot = conflictSnapshot(account: initial.account.id, voyage: voyage.serverId!, ids: ids)
  for invalid in [true, false] {
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(initial)
    let repository = LocalRepository(store: store)
    let before = try await store.read(accountId: initial.account.id)
    if invalid {
      await #expect(throws: WorkspaceError.invalidOrder) {
        try await repository.resolvePhotoConflict(
          accountId: initial.account.id, operationId: conflict.id,
          fields: snapshot.voyage.fields, snapshot: snapshot,
          choice: .arranged([.website(ids[0]), .website(ids[0])]))
      }
      #expect(try await store.read(accountId: initial.account.id) == before)
    } else {
      try await store.update(accountId: initial.account.id) { state in
        var other = OutboxOperation(
          accountId: state.account.id, generation: 0, voyageId: voyage.id,
          kind: .edit(voyage.fields))
        other.request = MutationRequest(method: "PATCH", path: "/submitted", revision: "7")
        other.state = .uncertain
        state.operations.append(other)
      }
      await #expect(throws: ValidationError.self) {
        try await repository.resolvePhotoConflict(
          accountId: initial.account.id, operationId: conflict.id,
          fields: snapshot.voyage.fields, snapshot: snapshot, choice: .website)
      }
      try await store.update(accountId: initial.account.id) { state in
        state.operations.removeAll {
          if case .edit = $0.kind { return true }
          return false
        }
      }
      try await repository.resolvePhotoConflict(
        accountId: initial.account.id, operationId: conflict.id,
        fields: voyage.fields, snapshot: snapshot,
        choice: .arranged(ids.reversed().map(PhotoConflictItem.website)))
      let saved = try await store.read(accountId: initial.account.id)
      #expect(saved.photos(for: voyage.id).compactMap(\.serverId) == ids.reversed())
      #expect(
        saved.operations.contains {
          if case .edit = $0.kind { return $0.expectedRevision == "12" }
          return false
        })
      let order = saved.operations.first {
        if case .order = $0.kind { return $0.state == .waiting }
        return false
      }
      #expect(order?.dependencies.count == 1)
    }
  }
}

@Test func explicitlyAddingAWebsiteDeletedPhotoCreatesFreshPhotoAndUploadIdentities() async throws {
  var state = workspaceWithPhotos(remote: true)
  state.photos[0].deleted = true
  let original = state.photos[0]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let id = try await LocalRepository(store: store).addAgain(
    accountId: state.account.id, photoId: original.id)
  let saved = try await store.read(accountId: state.account.id)
  let copy = try #require(saved.photos.first { $0.id == id })
  #expect(copy.uploadKey != original.uploadKey)
  #expect(copy.serverId == nil)
  #expect(copy.source == original.source)
  #expect(saved.photos.first { $0.id == original.id }?.deleted == true)
  #expect(saved.operations.contains { $0.kind == .upload(photoId: id, publicConsent: false) })
  #expect(!saved.voyages[0].confirmedPublished)
}
