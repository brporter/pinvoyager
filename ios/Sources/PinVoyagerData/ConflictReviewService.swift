import Foundation
import PinVoyagerDomain

public enum ConflictReviewResult: Equatable, Sendable {
  case savedLocally, resolved
  case needsReview(ConflictReview)
}

public struct ConflictReviewService: Sendable {
  private let store: any WorkspaceStore
  private let remote: any OwnerSnapshotLoading
  public init(store: any WorkspaceStore, remote: any OwnerSnapshotLoading) {
    self.store = store
    self.remote = remote
  }
  private func context(accountId: UUID, operationId: UUID) async throws -> (
    Workspace, OutboxOperation, LocalVoyage
  ) {
    let state = try await store.read(accountId: accountId)
    guard state.account.access != .locked else { throw WorkspaceError.locked }
    guard
      let operation = state.operations.first(where: {
        $0.id == operationId && $0.state == .conflict
      }),
      let voyage = state.voyages.first(where: { $0.id == operation.voyageId && !$0.deleted }),
      voyage.serverId != nil
    else { throw WorkspaceError.missingVoyage }
    return (state, operation, voyage)
  }
  public func load(accountId: UUID, operationId: UUID, online: Bool) async throws -> ConflictReview
  {
    let (state, _, voyage) = try await context(accountId: accountId, operationId: operationId)
    let saved = state.conflictReviews?.first { $0.voyageId == voyage.id }
    guard online else {
      guard var saved else { throw URLError(.notConnectedToInternet) }
      saved.generation = state.account.generation
      return saved
    }
    let snapshot = try await remote.ownerSnapshot(voyageId: voyage.serverId!, accountId: accountId)
    let review = try refreshed(saved, snapshot: snapshot, state: state, voyage: voyage)
    try await save(review, accountId: accountId, operationId: operationId)
    return review
  }
  private func refreshed(
    _ saved: ConflictReview?, snapshot: OwnerVoyageSnapshot, state: Workspace, voyage: LocalVoyage
  ) throws -> ConflictReview {
    let remote = snapshot.voyage
    guard remote.ownerId == state.account.id, remote.id == voyage.serverId else {
      throw WorkspaceError.wrongAccount
    }
    guard let revision = remote.revision,
      revision.range(of: "^[0-9]+$", options: .regularExpression) != nil
    else { throw WorkspaceError.malformedReceipt }
    let ids = remote.photos.sorted { $0.position < $1.position }.map(\.id)
    var review =
      saved
      ?? ConflictReview(
        voyageId: voyage.id, generation: state.account.generation,
        mine: voyage.fields, website: remote.fields, revision: revision,
        published: remote.published, websitePhotos: ids)
    if saved == nil, let base = voyage.serverFields {
      review.choices = [
        "title": FieldComparison(base: base.title, local: voyage.fields.title, remote: remote.title)
          .resolution,
        "description": FieldComparison(
          base: base.description, local: voyage.fields.description, remote: remote.description
        ).resolution,
        "granularity": FieldComparison(
          base: base.granularity, local: voyage.fields.granularity, remote: remote.granularity
        ).resolution,
      ]
    }
    if case .arranged(let previous) = review.photoChoice {
      let items =
        ids.map(PhotoConflictItem.website)
        + state.photos(for: voyage.id).filter { $0.serverId == nil }.map { .local($0.id) }
      review.photoChoice = .arranged(
        previous.filter { items.contains($0) } + items.filter { !previous.contains($0) })
    }
    review.generation = state.account.generation
    review.website = remote.fields
    review.revision = revision
    review.published = remote.published
    review.websitePhotos = ids
    return review
  }
  public func save(_ review: ConflictReview, accountId: UUID, operationId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      guard state.account.access != .locked, state.account.generation == review.generation else {
        throw WorkspaceError.locked
      }
      guard
        state.operations.contains(where: {
          $0.id == operationId && $0.voyageId == review.voyageId && $0.state == .conflict
        })
      else { throw WorkspaceError.missingVoyage }
      var reviews = state.conflictReviews ?? []
      reviews.removeAll { $0.voyageId == review.voyageId }
      reviews.append(review)
      state.conflictReviews = reviews
    }
  }
  public func apply(_ review: ConflictReview, accountId: UUID, operationId: UUID, online: Bool)
    async throws -> ConflictReviewResult
  {
    let (state, operation, voyage) = try await context(
      accountId: accountId, operationId: operationId)
    guard voyage.id == review.voyageId, state.account.generation == review.generation else {
      throw WorkspaceError.locked
    }
    try await save(review, accountId: accountId, operationId: operationId)
    guard online else { return .savedLocally }
    let snapshot = try await remote.ownerSnapshot(voyageId: voyage.serverId!, accountId: accountId)
    let latest = try refreshed(review, snapshot: snapshot, state: state, voyage: voyage)
    guard latest.revision == review.revision, latest.website == review.website,
      latest.websitePhotos == review.websitePhotos, latest.published == review.published
    else {
      try await save(latest, accountId: accountId, operationId: operationId)
      return .needsReview(latest)
    }
    let repository = LocalRepository(store: store)
    if case .unpublish = operation.kind {
      guard let makePrivate = review.makePrivate else {
        throw ValidationError(
          field: "visibility",
          message: "Choose whether to keep website visibility or make this voyage private.")
      }
      try await repository.resolveVisibilityConflict(
        accountId: accountId, operationId: operationId,
        snapshot: snapshot, makePrivate: makePrivate, expectedGeneration: review.generation)
      return .resolved
    }
    guard review.fieldsChosen else {
      throw ValidationError(
        field: "conflict", message: "Choose which version of each field to keep.")
    }
    switch operation.kind {
    case .order, .removePhoto:
      guard let choice = review.photoChoice else { throw WorkspaceError.invalidOrder }
      try await repository.resolvePhotoConflict(
        accountId: accountId, operationId: operationId,
        fields: review.resolvedFields, snapshot: snapshot, choice: choice,
        expectedGeneration: review.generation)
    default:
      try await repository.resolveMetadataConflict(
        accountId: accountId, operationId: operationId,
        fields: review.resolvedFields, websiteFields: review.website, revision: review.revision,
        expectedGeneration: review.generation)
    }
    return .resolved
  }
}
