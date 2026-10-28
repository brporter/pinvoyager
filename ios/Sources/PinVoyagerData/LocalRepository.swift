import Foundation
import PinVoyagerDomain

public actor LocalRepository {
  public let store: any WorkspaceStore
  public init(store: any WorkspaceStore) { self.store = store }

  private static func requireAccess(_ state: Workspace, voyageId: UUID? = nil) throws {
    guard state.account.access != .locked else { throw WorkspaceError.locked }
    if let voyageId {
      guard state.voyages.contains(where: { $0.id == voyageId && !$0.deleted }) else {
        throw WorkspaceError.missingVoyage
      }
      guard
        !state.publicationIntents.contains(where: {
          $0.voyageId == voyageId && state.publicationMayProceed($0)
        })
      else { throw WorkspaceError.publicationArmed }
    }
  }

  @discardableResult
  public func createVoyage(accountId: UUID, fields: VoyageFields, at: Date = Date()) async throws
    -> UUID
  {
    let fields = try fields.validated(isNew: true)
    let voyage = LocalVoyage(ownerId: accountId, fields: fields, at: at)
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      state.voyages.append(voyage)
      state.scratch = nil
      Self.enqueue(&state, voyageId: voyage.id, kind: .create(fields))
    }
    return voyage.id
  }

  public func saveWorkingCopy(accountId: UUID, voyageId: UUID, fields: VoyageFields) async throws {
    try await store.update(accountId: accountId) { state in
      guard state.account.access != .locked,
        let voyage = state.voyages.first(where: { $0.id == voyageId })
      else { throw WorkspaceError.locked }
      if let index = state.workingCopies.firstIndex(where: { $0.voyageId == voyageId }) {
        state.workingCopies[index].fields = fields
      } else {
        state.workingCopies.append(VoyageWorkingCopy(voyage: voyage, fields: fields))
      }
    }
  }

  public func saveScratch(accountId: UUID, fields: VoyageFields?) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      state.scratch = fields
    }
  }

  public func discardWorkingCopy(accountId: UUID, voyageId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      state.workingCopies.removeAll { $0.voyageId == voyageId }
    }
  }

  public func editVoyage(accountId: UUID, voyageId: UUID, fields: VoyageFields, at: Date = Date())
    async throws
  {
    let fields = try fields.validated(isNew: false)
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state, voyageId: voyageId)
      let index = state.voyages.firstIndex { $0.id == voyageId }!
      state.voyages[index].fields = fields
      state.voyages[index].localSequence += 1
      state.voyages[index].updatedAt = at
      state.workingCopies.removeAll { $0.voyageId == voyageId }
      Self.invalidateLocalConsent(&state, voyageId: voyageId)
      if let create = state.operations.firstIndex(where: { operation in
        guard operation.voyageId == voyageId, operation.state == .waiting, operation.request == nil
        else { return false }
        if case .create = operation.kind { return true }
        return false
      }) {
        state.operations[create].kind = .create(fields)
      } else {
        Self.enqueue(&state, voyageId: voyageId, kind: .edit(fields))
      }
    }
  }

  public func addPhoto(
    accountId: UUID, photo: LocalPhoto, publicConsent: Bool = false, expectedGeneration: Int? = nil
  ) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.addPhoto(
        &state, photo: photo, publicConsent: publicConsent,
        expectedGeneration: expectedGeneration)
    }
  }

  private static func addPhoto(
    _ state: inout Workspace, photo: LocalPhoto,
    publicConsent: Bool, expectedGeneration: Int? = nil
  ) throws {
    guard photo.source != nil else {
      throw ValidationError(
        field: "photo", message: "Save the file before adding it to the upload queue.")
    }
    try Self.requireAccess(state, voyageId: photo.voyageId)
    guard expectedGeneration == nil || state.account.generation == expectedGeneration else {
      throw WorkspaceError.locked
    }
    let voyage = state.voyages.first { $0.id == photo.voyageId }!
    guard !voyage.confirmedPublished || publicConsent else {
      throw ValidationError(
        field: "visibility",
        message: "Confirm that these photos will become public as they upload.")
    }
    let replaces = state.photos.first(where: { $0.id == photo.id })?.replacesPhotoId
    if let originalId = replaces {
      guard state.photos.contains(where: { $0.id == originalId && !$0.deleted }) else {
        throw ValidationError(
          field: "photo",
          message:
            "The original was removed while its replacement was being prepared. Your files have been kept."
        )
      }
      try Self.removePhoto(&state, photoId: originalId)
    }
    var saved = photo
    saved.transfer = .saved
    if let index = state.photos.firstIndex(where: { $0.id == photo.id }) {
      guard state.photos[index].source == nil, !state.photos[index].deleted else { return }
      saved.order = state.photos[index].order
      saved.importBatchId = state.photos[index].importBatchId
      saved.cellularException = state.photos[index].cellularException
      saved.importPublicConsent = state.photos[index].importPublicConsent
      state.photos[index] = saved
    } else {
      saved.order = (state.photos(for: photo.voyageId).map(\.order).max() ?? -1) + 1
      state.photos.append(saved)
    }
    Self.invalidateLocalConsent(&state, voyageId: photo.voyageId)
    Self.refreshUnsubmittedOrder(&state, voyageId: photo.voyageId)
    Self.enqueue(
      &state, voyageId: photo.voyageId,
      kind: .upload(photoId: photo.id, publicConsent: publicConsent))
    if replaces != nil {
      Self.enqueue(
        &state, voyageId: photo.voyageId,
        kind: .order(state.photos(for: photo.voyageId).map(\.id)))
    }

  }

  public func beginReplacement(
    accountId: UUID, photoId: UUID, filename: String, publicConsent: Bool = false,
    expectedGeneration: Int? = nil
  ) async throws -> UUID {
    let id = UUID()
    try await store.update(accountId: accountId) { state in
      guard let selected = state.photos.firstIndex(where: { $0.id == photoId && !$0.deleted })
      else { throw FileVaultError.missingSource }
      let old = state.photos[selected]
      let originalId = old.source == nil ? old.replacesPhotoId ?? old.id : old.id
      guard let original = state.photos.first(where: { $0.id == originalId && !$0.deleted }) else {
        throw FileVaultError.missingSource
      }
      try Self.requireAccess(state, voyageId: original.voyageId)
      guard expectedGeneration == nil || state.account.generation == expectedGeneration else {
        throw WorkspaceError.locked
      }
      guard
        state.voyages.first(where: { $0.id == original.voyageId })?.confirmedPublished != true
          || publicConsent
      else {
        throw ValidationError(
          field: "visibility",
          message: "Confirm that the replacement will become public as it uploads.")
      }
      if originalId != old.id {
        state.photos[selected].deleted = true
        state.photos[selected].transfer = .removed
      }
      var pending = LocalPhoto(id: id, voyageId: original.voyageId, filename: filename)
      pending.replacesPhotoId = originalId
      pending.importPublicConsent = publicConsent
      pending.importBatchId = UUID()
      pending.order = original.order
      state.photos.append(pending)
      Self.invalidateLocalConsent(&state, voyageId: original.voyageId)
    }
    return id
  }

  public func beginImport(
    accountId: UUID, voyageId: UUID, filename: String, publicConsent: Bool = false
  ) async throws -> UUID {
    try await beginImports(
      accountId: accountId, voyageId: voyageId, filenames: [filename], publicConsent: publicConsent)[
        0]
  }

  public func beginImports(
    accountId: UUID, voyageId: UUID, filenames: [String], publicConsent: Bool = false,
    expectedGeneration: Int? = nil, cellularException: Bool = false
  ) async throws -> [UUID] {
    guard filenames.count <= 100 else {
      throw ValidationError(field: "photos", message: "Choose at most 100 photographs at once.")
    }
    let batchId = UUID()
    let photos = filenames.map { name in LocalPhoto(voyageId: voyageId, filename: name) }
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state, voyageId: voyageId)
      guard expectedGeneration == nil || state.account.generation == expectedGeneration else {
        throw WorkspaceError.locked
      }
      guard
        state.voyages.first(where: { $0.id == voyageId })?.confirmedPublished != true
          || publicConsent
      else {
        throw ValidationError(
          field: "visibility",
          message: "Confirm that these photos will become public as they upload.")
      }
      let firstOrder = (state.photos(for: voyageId).map(\.order).max() ?? -1) + 1
      for (index, pending) in photos.enumerated() {
        var photo = pending
        photo.importPublicConsent = publicConsent
        photo.importBatchId = batchId
        photo.cellularException = cellularException ? true : nil
        photo.order = firstOrder + index
        state.photos.append(photo)
      }
      if !photos.isEmpty { Self.invalidateLocalConsent(&state, voyageId: voyageId) }
    }
    return photos.map(\.id)
  }

  public func markImportMissing(accountId: UUID, photoId: UUID, message: String) async throws {
    try await store.update(accountId: accountId) { state in
      guard state.account.access != .locked,
        let index = state.photos.firstIndex(where: { $0.id == photoId && $0.source == nil })
      else { return }
      state.photos[index].transfer = .needsSource
      state.photos[index].error = message
    }
  }

  public func recoverInterruptedImports(accountId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      for index in state.photos.indices where state.photos[index].transfer == .acquiring {
        state.photos[index].transfer = .needsSource
        state.photos[index].error = "Choose this photo again. Its acquisition was interrupted."
      }
    }
  }

  public func setUploadsPaused(accountId: UUID, voyageId: UUID, paused: Bool) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      guard let index = state.voyages.firstIndex(where: { $0.id == voyageId && !$0.deleted }) else {
        throw WorkspaceError.missingVoyage
      }
      state.voyages[index].uploadsPaused = paused
    }
  }

  public func reorder(accountId: UUID, voyageId: UUID, photoIds: [UUID]) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state, voyageId: voyageId)
      guard Set(photoIds).count == photoIds.count,
        Set(photoIds) == Set(state.photos(for: voyageId).map(\.id)), photoIds.count <= 10_000
      else { throw WorkspaceError.invalidOrder }
      for (order, id) in photoIds.enumerated() {
        if let index = state.photos.firstIndex(where: { $0.id == id }) {
          state.photos[index].order = order
        }
      }
      Self.invalidateLocalConsent(&state, voyageId: voyageId)
      Self.cancelUnsubmittedOrders(&state, voyageId: voyageId)
      Self.enqueue(&state, voyageId: voyageId, kind: .order(photoIds))
    }
  }

  public func removePhoto(accountId: UUID, photoId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.removePhoto(&state, photoId: photoId)
    }
  }
  public func removePhotos(accountId: UUID, voyageId: UUID, photoIds: Set<UUID>) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state, voyageId: voyageId)
      guard photoIds.isSubset(of: Set(state.photos(for: voyageId).map(\.id))) else {
        throw WorkspaceError.invalidOrder
      }
      for id in state.photos(for: voyageId).map(\.id) where photoIds.contains(id) {
        try Self.removePhoto(&state, photoId: id)
      }
    }
  }
  private static func removePhoto(_ state: inout Workspace, photoId: UUID) throws {
    guard let index = state.photos.firstIndex(where: { $0.id == photoId }) else { return }
    let photo = state.photos[index]
    try Self.requireAccess(state, voyageId: photo.voyageId)
    state.photos[index].deleted = true
    state.photos[index].cellularException = nil
    state.photos[index].transfer = .removing
    for op in state.operations.indices {
      if case .upload(let id, _) = state.operations[op].kind, id == photoId,
        state.operations[op].request == nil
      {
        state.operations[op].state = .canceled
      }
    }
    Self.invalidateLocalConsent(&state, voyageId: photo.voyageId)
    Self.refreshUnsubmittedOrder(&state, voyageId: photo.voyageId)
    Self.enqueue(
      &state, voyageId: photo.voyageId,
      kind: .removePhoto(photoId: photoId, uploadKey: photo.uploadKey))
  }

  public func undoPhotoRemoval(accountId: UUID, photoId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.undoPhotoRemoval(&state, photoId: photoId)
    }
  }
  public func undoPhotoRemovals(accountId: UUID, voyageId: UUID, photoIds: Set<UUID>) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state, voyageId: voyageId)
      guard photoIds.isSubset(of: Set(state.photos.filter { $0.voyageId == voyageId }.map(\.id)))
      else { throw WorkspaceError.invalidOrder }
      for id in photoIds { try Self.undoPhotoRemoval(&state, photoId: id) }
    }
  }
  private static func undoPhotoRemoval(_ state: inout Workspace, photoId: UUID) throws {
    guard let photo = state.photos.firstIndex(where: { $0.id == photoId && $0.deleted }) else {
      return
    }
    try Self.requireAccess(state, voyageId: state.photos[photo].voyageId)
    guard
      let removal = state.operations.lastIndex(where: {
        if case .removePhoto(let id, _) = $0.kind { return id == photoId }
        return false
      }), state.operations[removal].request == nil, state.operations[removal].state == .waiting
    else {
      throw ValidationError(
        field: "photo",
        message:
          "Removal has already been submitted. Its server outcome must be checked before restoring this photo."
      )
    }
    state.operations[removal].state = .canceled
    state.photos[photo].deleted = false
    state.photos[photo].transfer = state.photos[photo].serverId != nil ? .accepted : .saved
    for index in state.operations.indices
    where state.operations[index].request == nil && state.operations[index].state == .canceled {
      if case .upload(let id, _) = state.operations[index].kind, id == photoId {
        state.operations[index].state = .waiting
      }
    }
    Self.invalidateLocalConsent(&state, voyageId: state.photos[photo].voyageId)
    Self.refreshUnsubmittedOrder(&state, voyageId: state.photos[photo].voyageId)
  }

  public func deleteVoyage(accountId: UUID, voyageId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      guard state.account.access != .locked,
        let index = state.voyages.firstIndex(where: { $0.id == voyageId })
      else { throw WorkspaceError.locked }
      guard !state.voyages[index].deleted else { return }
      state.voyages[index].deletionUndo = VoyageDeletionUndo(
        photoIds: Set(state.photos(for: voyageId).map(\.id)),
        operationStates: Dictionary(
          uniqueKeysWithValues: state.operations.filter {
            $0.voyageId == voyageId && $0.request == nil
              && ![.applied, .canceled].contains($0.state)
          }.map { ($0.id, $0.state) }))
      state.voyages[index].deleted = true
      for photo in state.photos.indices where state.photos[photo].voyageId == voyageId {
        state.photos[photo].deleted = true
        state.photos[photo].cellularException = nil
      }
      for op in state.operations.indices
      where state.operations[op].voyageId == voyageId && state.operations[op].request == nil
        && ![.applied, .canceled].contains(state.operations[op].state)
      {
        state.operations[op].state = .canceled
      }
      Self.enqueue(&state, voyageId: voyageId, kind: .deleteVoyage)
    }
  }

  public func undoVoyageDeletion(accountId: UUID, voyageId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      guard state.canUndoVoyageDeletion(voyageId),
        let index = state.voyages.firstIndex(where: { $0.id == voyageId }),
        let snapshot = state.voyages[index].deletionUndo
      else {
        throw ValidationError(
          field: "voyage",
          message:
            "Deletion has already been submitted. Check its server outcome before restoring this voyage."
        )
      }
      state.voyages[index].deleted = false
      state.voyages[index].deletionUndo = nil
      for index in state.photos.indices where snapshot.photoIds.contains(state.photos[index].id) {
        state.photos[index].deleted = false
      }
      for index in state.operations.indices {
        if state.operations[index].voyageId == voyageId
          && state.operations[index].kind == .deleteVoyage && state.operations[index].request == nil
          && state.operations[index].state == .waiting
        {
          state.operations[index].state = .canceled
        } else if let previous = snapshot.operationStates[state.operations[index].id],
          state.operations[index].request == nil && state.operations[index].state == .canceled
        {
          state.operations[index].state = previous
        }
      }
    }
  }

  @discardableResult
  public func queuePublication(accountId: UUID, voyageId: UUID, at: Date = Date()) async throws
    -> UUID
  {
    let id = UUID()
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state, voyageId: voyageId)
      guard state.account.profile.displayName != nil else {
        throw ValidationError(
          field: "display_name", message: "Choose a display name before publication.")
      }
      let voyage = state.voyages.first { $0.id == voyageId }!
      let intent = try PublicationIntent.review(
        voyage: voyage, photos: state.photos(for: voyageId), at: at)
      Self.invalidateLocalConsent(&state, voyageId: voyageId)
      Self.cancelUnsubmittedOrders(&state, voyageId: voyageId)
      let rejected = Set(
        state.operations.compactMap { operation -> UUID? in
          guard operation.voyageId == voyageId, operation.receiptRejected == true,
            case .publish(let id) = operation.kind
          else { return nil }
          return id
        })
      Self.cancelLocalPublicationOperations(&state, intentIds: rejected)
      state.publicationIntents.append(intent)
      Self.enqueue(&state, id: id, voyageId: voyageId, kind: .publish(intentId: intent.id))
    }
    return id
  }

  public func cancelPublication(accountId: UUID, intentId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      guard let index = state.publicationIntents.firstIndex(where: { $0.id == intentId }),
        state.account.access != .locked
      else { throw WorkspaceError.locked }
      if state.publicationIntents[index].serverId == nil
        && !state.operations.contains(where: {
          if case .publish(let id) = $0.kind {
            return id == intentId && $0.request != nil && $0.receiptRejected != true
          }
          return false
        })
      {
        state.publicationIntents[index].status = .canceled
        state.publicationIntents[index].cancellationPending = false
        Self.cancelLocalPublicationOperations(&state, intentIds: [intentId])
      } else {
        state.publicationIntents[index].cancellationPending = true
        for op in state.operations.indices {
          if case .publish(let id) = state.operations[op].kind, id == intentId,
            state.operations[op].request != nil,
            [.attention, .conflict].contains(state.operations[op].state)
          {
            state.operations[op].state = .uncertain
            state.operations[op].nextAttempt = .distantPast
          }
        }
        Self.enqueue(
          &state, voyageId: state.publicationIntents[index].voyageId,
          kind: .cancelPublication(intentId: intentId))
      }
    }
  }

  public func unpublish(accountId: UUID, voyageId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      guard state.account.access != .locked else { throw WorkspaceError.locked }
      guard state.voyages.contains(where: { $0.id == voyageId && !$0.deleted }) else {
        throw WorkspaceError.missingVoyage
      }
      Self.enqueue(&state, voyageId: voyageId, kind: .unpublish)
    }
  }

  public func retry(accountId: UUID, operationId: UUID) async throws {
    try await store.update(accountId: accountId) { state in
      guard state.account.access != .locked,
        let index = state.operations.firstIndex(where: { $0.id == operationId }),
        state.operations[index].state == .attention
      else { return }
      state.operations[index].attempts = 0
      state.operations[index].nextAttempt = .distantPast
      state.operations[index].state = state.operations[index].request == nil ? .waiting : .uncertain
      state.operations[index].lastError = nil
    }
  }

  public func lock(accountId: UUID, pauseSync: Bool = false) async throws {
    try await store.update(accountId: accountId) { state in
      state.account.access = .locked
      state.account.generation += 1
      if pauseSync { state.policy.paused = true }
    }
  }

  public func setCellularException(accountId: UUID, batchId: UUID, allowed: Bool) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      var changed = false
      for index in state.photos.indices
      where state.photos[index].importBatchId == batchId && !state.photos[index].deleted
        && state.photos[index].serverId == nil
      {
        if (state.photos[index].cellularException == true) != allowed {
          state.photos[index].cellularException = allowed ? true : nil
          changed = true
        }
      }
      if changed { state.account.generation += 1 }
    }
  }

  public func setPolicy(accountId: UUID, policy: SyncPolicy) async throws {
    try await updatePolicy(accountId: accountId) { $0 = policy }
  }

  public func setPreference(accountId: UUID, preference: SyncPreference, value: Bool) async throws {
    try await updatePolicy(accountId: accountId) { $0[keyPath: preference.keyPath] = value }
  }

  private func updatePolicy(
    accountId: UUID,
    _ change: @escaping @Sendable (inout SyncPolicy) -> Void
  ) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      var policy = state.policy
      change(&policy)
      if policy.wifiOnly != state.policy.wifiOnly
        || policy.allowsConstrained != state.policy.allowsConstrained
        || policy.paused != state.policy.paused
        || policy.areUploadsPaused != state.policy.areUploadsPaused
      {
        state.account.generation += 1
      }
      state.policy = policy
    }
  }

  public func setBlocked(accountId: UUID, userId: UUID, blocked: Bool) async throws {
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      guard userId != accountId else {
        throw ValidationError(field: "block", message: "You cannot block your own account.")
      }
      if blocked { state.blockedAccounts.insert(userId) }
      Self.enqueue(&state, voyageId: nil, kind: .block(userId: userId, blocked: blocked))
    }
  }

  @discardableResult public func report(
    accountId: UUID, target: String, reason: String, details: String
  ) async throws -> UUID {
    guard details.unicodeScalars.count <= 2_000 else {
      throw ValidationError(field: "details", message: "Use at most 2,000 characters.")
    }
    let id = UUID()
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      Self.enqueue(
        &state, id: id, voyageId: nil,
        kind: .report(target: target, reason: reason, details: details))
    }
    return id
  }

  public func resolveMetadataConflict(
    accountId: UUID, operationId: UUID, fields: VoyageFields, websiteFields: VoyageFields,
    revision: String, expectedGeneration: Int? = nil
  ) async throws {
    let fields = try fields.validated(isNew: false)
    guard revision.range(of: "^[0-9]+$", options: .regularExpression) != nil else {
      throw WorkspaceError.malformedReceipt
    }
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      guard expectedGeneration == nil || state.account.generation == expectedGeneration else {
        throw WorkspaceError.locked
      }
      guard
        let operation = state.operations.firstIndex(where: {
          $0.id == operationId && $0.state == .conflict
        }),
        case .edit = state.operations[operation].kind,
        let voyageId = state.operations[operation].voyageId,
        let voyage = state.voyages.firstIndex(where: { $0.id == voyageId })
      else { throw WorkspaceError.missingVoyage }
      state.operations[operation].state = .canceled
      state.voyages[voyage].fields = fields
      state.voyages[voyage].serverFields = websiteFields
      state.voyages[voyage].revision = revision
      state.voyages[voyage].localSequence += 1
      Self.invalidateLocalConsent(&state, voyageId: voyageId)
      Self.enqueue(&state, voyageId: voyageId, kind: .edit(fields))
    }
  }

  public func resolveVisibilityConflict(
    accountId: UUID, operationId: UUID,
    snapshot: OwnerVoyageSnapshot, makePrivate: Bool, expectedGeneration: Int
  ) async throws {
    let remote = snapshot.voyage
    guard remote.ownerId == accountId, let revision = remote.revision,
      revision.range(of: "^[0-9]+$", options: .regularExpression) != nil
    else { throw WorkspaceError.malformedReceipt }
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      guard state.account.generation == expectedGeneration else { throw WorkspaceError.locked }
      guard
        let operation = state.operations.firstIndex(where: {
          $0.id == operationId && $0.state == .conflict
        }),
        case .unpublish = state.operations[operation].kind,
        let voyage = state.voyages.firstIndex(where: {
          $0.id == state.operations[operation].voyageId && $0.serverId == remote.id
        })
      else { throw WorkspaceError.missingVoyage }
      state.operations[operation].state = .canceled
      state.voyages[voyage].revision = revision
      state.voyages[voyage].serverFields = remote.fields
      state.voyages[voyage].confirmedPublished = remote.published
      if makePrivate && remote.published {
        Self.invalidateLocalConsent(&state, voyageId: state.voyages[voyage].id)
        Self.enqueue(&state, voyageId: state.voyages[voyage].id, kind: .unpublish)
      }
    }
  }

  @discardableResult public func addAgain(
    accountId: UUID, photoId: UUID, publicConsent: Bool = false
  ) async throws -> UUID {
    let id = UUID()
    try await store.update(accountId: accountId) { state in
      guard let original = state.photos.first(where: { $0.id == photoId }), original.source != nil
      else { throw FileVaultError.missingSource }
      var photo = LocalPhoto(id: id, voyageId: original.voyageId, filename: original.filename)
      photo.source = original.source
      photo.importBatchId = UUID()
      photo.previewPath = original.previewPath
      photo.coordinate = original.coordinate
      photo.capturedAt = original.capturedAt
      photo.width = original.width
      photo.height = original.height
      try Self.addPhoto(&state, photo: photo, publicConsent: publicConsent)
    }
    return id
  }

  public func resolvePhotoConflict(
    accountId: UUID, operationId: UUID, fields: VoyageFields,
    snapshot: OwnerVoyageSnapshot, choice: PhotoConflictChoice, expectedGeneration: Int? = nil
  ) async throws {
    let fields = try fields.validated(isNew: false)
    let remote = snapshot.voyage
    guard remote.ownerId == accountId else { throw WorkspaceError.wrongAccount }
    guard let revision = remote.revision,
      revision.range(of: "^[0-9]+$", options: .regularExpression) != nil,
      snapshot.photos.count == remote.photos.count,
      Set(snapshot.photos.map(\.id)) == Set(remote.photos.map(\.id))
    else {
      throw WorkspaceError.malformedReceipt
    }
    try await store.update(accountId: accountId) { state in
      guard
        let operation = state.operations.first(where: {
          $0.id == operationId && $0.state == .conflict
        }),
        let voyageId = operation.voyageId,
        let voyage = state.voyages.firstIndex(where: {
          $0.id == voyageId && $0.serverId == remote.id
        })
      else {
        throw WorkspaceError.missingVoyage
      }
      try Self.requireAccess(state, voyageId: voyageId)
      guard expectedGeneration == nil || state.account.generation == expectedGeneration else {
        throw WorkspaceError.locked
      }
      switch operation.kind {
      case .order, .removePhoto: break
      default: throw WorkspaceError.invalidOrder
      }
      guard
        !state.operations.contains(where: {
          $0.voyageId == voyageId && $0.id != operationId && $0.request != nil
            && ![.applied, .canceled].contains($0.state)
        })
      else {
        throw ValidationError(
          field: "conflict",
          message: "Check the other submitted changes before resolving this photo list.")
      }
      let local = state.photos(for: voyageId).filter { $0.serverId == nil }
      let website = remote.photos.sorted {
        ($0.position, $0.id.uuidString) < ($1.position, $1.id.uuidString)
      }
      let websiteItems = website.map { PhotoConflictItem.website($0.id) }
      let allItems = websiteItems + local.map { PhotoConflictItem.local($0.id) }
      let items: [PhotoConflictItem]
      switch choice {
      case .website: items = websiteItems
      case .websiteAppendingLocal: items = allItems
      case .arranged(let order):
        guard order.count == allItems.count, Set(order) == Set(allItems) else {
          throw WorkspaceError.invalidOrder
        }
        items = order
      }
      let included = Set(items)
      let metadata = Dictionary(
        snapshot.photos.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
      for index in state.operations.indices
      where state.operations[index].voyageId == voyageId
        && ![.applied, .canceled].contains(state.operations[index].state)
      {
        switch state.operations[index].kind {
        case .order, .removePhoto, .edit: state.operations[index].state = .canceled
        case .upload(let id, _) where !included.contains(.local(id)):
          state.operations[index].state = .canceled
        default: break
        }
      }
      for index in state.photos.indices where state.photos[index].voyageId == voyageId {
        let photo = state.photos[index]
        let item = photo.serverId.map(PhotoConflictItem.website) ?? .local(photo.id)
        state.photos[index].deleted = !included.contains(item)
      }
      var ordered: [UUID] = []
      for (position, item) in items.enumerated() {
        let index: Int
        switch item {
        case .local(let id):
          guard
            let found = state.photos.firstIndex(where: { $0.id == id && $0.voyageId == voyageId })
          else { throw WorkspaceError.invalidOrder }
          index = found
        case .website(let id):
          if let found = state.photos.firstIndex(where: {
            $0.serverId == id && $0.voyageId == voyageId
          }) {
            index = found
          } else {
            var photo = LocalPhoto(voyageId: voyageId, filename: "Website photo \(position + 1)")
            photo.serverId = id
            photo.transfer = .accepted
            state.photos.append(photo)
            index = state.photos.count - 1
          }
          if let details = metadata[id] {
            state.photos[index].coordinate = details.coordinate
            state.photos[index].capturedAt = details.capturedAt
            state.photos[index].width = details.width
            state.photos[index].height = details.height
            state.photos[index].variant = details.variantStatus
          }
        }
        state.photos[index].order = position
        ordered.append(state.photos[index].id)
      }
      state.voyages[voyage].fields = fields
      state.voyages[voyage].serverFields = remote.fields
      state.voyages[voyage].revision = revision
      state.voyages[voyage].confirmedPublished = remote.published
      state.voyages[voyage].localSequence += 1
      state.workingCopies.removeAll { $0.voyageId == voyageId }
      Self.invalidateLocalConsent(&state, voyageId: voyageId)
      if fields != remote.fields { Self.enqueue(&state, voyageId: voyageId, kind: .edit(fields)) }
      if case .website = choice { return }
      Self.enqueue(&state, voyageId: voyageId, kind: .order(ordered))
    }
  }

  @discardableResult
  func recoverAsNewDraft(
    accountId: UUID, voyageId: UUID, sources: [RecoveredPhotoSource], expectedGeneration: Int
  ) async throws -> UUID {
    let newId = UUID()
    try await store.update(accountId: accountId) { state in
      try Self.requireAccess(state)
      guard let original = state.voyages.first(where: { $0.id == voyageId }) else {
        throw WorkspaceError.missingVoyage
      }
      let newVoyage = LocalVoyage(id: newId, ownerId: accountId, fields: original.fields)
      guard state.account.generation == expectedGeneration,
        sources.allSatisfy({ source in state.photos(for: voyageId).contains(source.original) })
      else { throw WorkspaceError.locked }
      state.voyages.append(newVoyage)
      Self.enqueue(&state, voyageId: newId, kind: .create(newVoyage.fields))
      if let working = state.workingCopies.first(where: { $0.voyageId == voyageId }) {
        state.workingCopies.append(VoyageWorkingCopy(voyage: newVoyage, fields: working.fields))
      }
      for (order, retained) in sources.enumerated() {
        let originalPhoto = retained.original
        var photo = LocalPhoto(
          voyageId: newId,
          filename: retained.isOriginal ? originalPhoto.filename : "Recovered processed copy")
        photo.source = retained.source
        photo.previewPath = originalPhoto.previewPath
        photo.coordinate = retained.isOriginal ? originalPhoto.coordinate : nil
        photo.capturedAt = retained.isOriginal ? originalPhoto.capturedAt : nil
        photo.width = originalPhoto.width
        photo.height = originalPhoto.height
        photo.order = order
        photo.transfer = .saved
        state.photos.append(photo)
        Self.enqueue(
          &state, voyageId: newId, kind: .upload(photoId: photo.id, publicConsent: false))
      }
    }
    return newId
  }

  static func cancelLocalPublicationOperations(
    _ state: inout Workspace, intentIds: Set<UUID>
  ) {
    for index in state.operations.indices {
      switch state.operations[index].kind {
      case .publish(let id) where intentIds.contains(id):
        if state.operations[index].request == nil || state.operations[index].receiptRejected == true
        {
          state.operations[index].state = .canceled
        }
      case .cancelPublication(let id)
      where intentIds.contains(id) && state.operations[index].request == nil:
        state.operations[index].state = .canceled
      default: break
      }
    }
  }

  private static func invalidateLocalConsent(_ state: inout Workspace, voyageId: UUID) {
    for index in state.publicationIntents.indices
    where state.publicationIntents[index].voyageId == voyageId
      && state.publicationIntents[index].status == .queued
    {
      state.publicationIntents[index].status = .needsReview
      state.publicationIntents[index].cancellationPending = false
      let id = state.publicationIntents[index].id
      Self.cancelLocalPublicationOperations(&state, intentIds: [id])
    }
  }

  private static func refreshUnsubmittedOrder(_ state: inout Workspace, voyageId: UUID) {
    let order = state.photos(for: voyageId).map(\.id)
    for index in state.operations.indices
    where state.operations[index].voyageId == voyageId && state.operations[index].state == .waiting
      && state.operations[index].request == nil
    {
      if case .order = state.operations[index].kind { state.operations[index].kind = .order(order) }
    }
  }

  private static func cancelUnsubmittedOrders(_ state: inout Workspace, voyageId: UUID) {
    for index in state.operations.indices
    where state.operations[index].voyageId == voyageId && state.operations[index].request == nil {
      if case .order = state.operations[index].kind { state.operations[index].state = .canceled }
    }
  }

  private static func enqueue(
    _ state: inout Workspace, id: UUID = UUID(), voyageId: UUID?, kind: OperationKind
  ) {
    var successor = OutboxOperation(
      id: id, accountId: state.account.id, generation: state.account.generation, voyageId: voyageId,
      kind: kind, expectedRevision: state.voyages.first { $0.id == voyageId }?.revision)
    let predecessors = state.operations.filter {
      $0.scope == successor.scope && ![.applied, .canceled].contains($0.state)
    }
    let dependencies = Set(
      predecessors.compactMap { operation -> UUID? in
        // Publication must be registered before future uploads; deletions must resolve uncertain creates.
        if case .create = operation.kind { return operation.id }
        if kind.priority == 0 {
          if case .upload = operation.kind { return nil }
          return operation.request == nil ? nil : operation.id
        }
        if case .upload = operation.kind { return nil }
        if case .order = operation.kind {
          if case .publish = kind, operation.request != nil { return operation.id }
          return nil
        }
        return operation.id
      })
    successor.dependencies = dependencies
    state.operations.append(successor)
  }
}
