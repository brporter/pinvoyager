import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

private actor ReviewedSnapshots: OwnerSnapshotLoading {
  var snapshots: [OwnerVoyageSnapshot]
  init(_ snapshots: [OwnerVoyageSnapshot]) { self.snapshots = snapshots }
  func ownerSnapshot(voyageId: UUID, accountId: UUID) throws -> OwnerVoyageSnapshot {
    guard !snapshots.isEmpty else { throw URLError(.notConnectedToInternet) }
    return snapshots.removeFirst()
  }
}

@Test func offlineConflictWorkingCopiesSurviveRestartAndRepeatedWebsiteChangesKeepChoices()
  async throws
{
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("workspace.sqlite")
  let store = try CoreDataWorkspaceStore(url: url)
  var initial = workspaceWithPhotos(remote: true)
  let account = initial.account.id
  let voyage = initial.voyages[0]
  var operation = OutboxOperation(
    accountId: account, generation: 0, voyageId: voyage.id, kind: .edit(voyage.fields),
    expectedRevision: "7")
  operation.state = .conflict
  operation.request = MutationRequest(
    method: "PATCH", path: "/saved-immutable-request", revision: "7")
  initial.operations = [operation]
  try await store.insert(initial)
  let snapshots = ReviewedSnapshots([
    conflictSnapshot(account: account, voyage: voyage.serverId!, revision: "12", ids: []),
    conflictSnapshot(account: account, voyage: voyage.serverId!, revision: "13", ids: []),
    conflictSnapshot(account: account, voyage: voyage.serverId!, revision: "13", ids: []),
  ])
  let service = ConflictReviewService(store: store, remote: snapshots)
  var review = try await service.load(accountId: account, operationId: operation.id, online: true)
  review.mine.title = "My explicit title"
  review.choices = ["title": .local, "description": .remote, "granularity": .local]
  #expect(
    try await service.apply(review, accountId: account, operationId: operation.id, online: false)
      == .savedLocally)
  let reopened = try CoreDataWorkspaceStore(url: url)
  let offline = ConflictReviewService(store: reopened, remote: ReviewedSnapshots([]))
  #expect(
    try await offline.load(accountId: account, operationId: operation.id, online: false) == review)
  #expect(try await reopened.read(accountId: account).operations.count == 1)
  let result = try await service.apply(
    review, accountId: account, operationId: operation.id, online: true)
  guard case .needsReview(let latest) = result else {
    Issue.record("A changed website revision requires another review")
    return
  }
  #expect(latest.revision == "13")
  #expect(latest.choices == review.choices)
  #expect(latest.mine == review.mine)
  #expect(try await store.read(accountId: account).operations.count == 1)
  #expect(
    try await service.apply(latest, accountId: account, operationId: operation.id, online: true)
      == .resolved)
  let saved = try await store.read(accountId: account)
  #expect(saved.voyages[0].fields.title == "My explicit title")
  #expect(saved.voyages[0].fields.description == "Website description")
  #expect(saved.operations.first { $0.id == operation.id }?.request == operation.request)
  #expect(saved.operations.contains { $0.id != operation.id && $0.expectedRevision == "13" })
  #expect(saved.conflictReviews?.first?.choices == review.choices)
  try await LocalRepository(store: store).lock(accountId: account)
  await #expect(throws: WorkspaceError.locked) {
    try await service.load(accountId: account, operationId: operation.id, online: false)
  }
}

@Test func visibilityConflictsRequireAnExplicitChoiceAndNeverPublishFromAnOldRequest() async throws
{
  for makePrivate in [true, false] {
    var initial = workspaceWithPhotos(remote: true)
    let account = initial.account.id
    let voyage = initial.voyages[0]
    var operation = OutboxOperation(
      accountId: account, generation: 0, voyageId: voyage.id, kind: .unpublish,
      expectedRevision: "7")
    operation.state = .conflict
    operation.request = MutationRequest(
      method: "PATCH", path: "/saved-visibility-request", revision: "7")
    initial.operations = [operation]
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(initial)
    let snapshot = conflictSnapshot(
      account: account, voyage: voyage.serverId!, revision: "14", ids: [], published: true)
    let service = ConflictReviewService(
      store: store, remote: ReviewedSnapshots([snapshot, snapshot, snapshot]))
    var review = try await service.load(accountId: account, operationId: operation.id, online: true)
    await #expect(throws: ValidationError.self) {
      try await service.apply(review, accountId: account, operationId: operation.id, online: true)
    }
    review.makePrivate = makePrivate
    #expect(
      try await service.apply(review, accountId: account, operationId: operation.id, online: true)
        == .resolved)
    let saved = try await store.read(accountId: account)
    #expect(saved.voyages[0].confirmedPublished)
    #expect(saved.operations.first { $0.id == operation.id }?.state == .canceled)
    let pending = saved.operations.filter { $0.id != operation.id && $0.state == .waiting }
    #expect(pending.count == (makePrivate ? 1 : 0))
    #expect(pending.allSatisfy { $0.kind == .unpublish && $0.expectedRevision == "14" })
  }
}

@Test func refreshedArrangementsKeepSurvivorsAndAppendNewMembersWithoutResurrectingDeletions()
  async throws
{
  var state = workspaceWithPhotos(remote: true)
  let voyage = state.voyages[0]
  var operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: voyage.id, kind: .order([]))
  operation.state = .conflict
  state.operations = [operation]
  let removed = UUID()
  let surviving = state.photos[0].serverId!
  let arrival = UUID()
  var review = ConflictReview(
    voyageId: voyage.id, generation: 0, mine: voyage.fields,
    website: voyage.fields, revision: "7", published: false, websitePhotos: [removed, surviving])
  review.photoChoice = .arranged([.website(removed), .website(surviving)])
  state.conflictReviews = [review]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let snapshot = conflictSnapshot(
    account: state.account.id, voyage: voyage.serverId!, revision: "8", ids: [arrival, surviving])
  let service = ConflictReviewService(store: store, remote: ReviewedSnapshots([snapshot]))
  let refreshed = try await service.load(
    accountId: state.account.id, operationId: operation.id, online: true)
  #expect(refreshed.photoChoice == .arranged([.website(surviving), .website(arrival)]))
  #expect(refreshed.mine == review.mine)
}

@Test func conflictReviewRejectsWrongOwnersStaleGenerationsAndUnchosenFields() async throws {
  var state = workspaceWithPhotos(remote: true)
  let voyage = state.voyages[0]
  var operation = OutboxOperation(
    accountId: state.account.id, generation: 0, voyageId: voyage.id, kind: .edit(voyage.fields))
  operation.state = .conflict
  state.operations = [operation]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let snapshot = conflictSnapshot(
    account: state.account.id, voyage: voyage.serverId!, revision: "8", ids: [])
  let wrongOwner = conflictSnapshot(
    account: UUID(), voyage: voyage.serverId!, revision: "8", ids: [])
  let service = ConflictReviewService(
    store: store, remote: ReviewedSnapshots([wrongOwner, snapshot, snapshot]))
  await #expect(throws: WorkspaceError.wrongAccount) {
    try await service.load(accountId: state.account.id, operationId: operation.id, online: true)
  }
  var review = try await service.load(
    accountId: state.account.id, operationId: operation.id, online: true)
  review.choices = [:]
  await #expect(throws: ValidationError.self) {
    try await service.apply(
      review, accountId: state.account.id, operationId: operation.id, online: true)
  }
  try await store.update(accountId: state.account.id) { $0.account.generation += 1 }
  await #expect(throws: WorkspaceError.locked) {
    try await service.save(review, accountId: state.account.id, operationId: operation.id)
  }
  await #expect(throws: WorkspaceError.locked) {
    try await service.apply(
      review, accountId: state.account.id, operationId: operation.id, online: false)
  }
}
