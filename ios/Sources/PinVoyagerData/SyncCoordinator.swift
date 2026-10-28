import Foundation
import PinVoyagerDomain

public enum OperationReceipt: Equatable, Sendable {
  case notSeen, inProgress
  case applied(targetId: UUID, revision: String, previousRevision: String? = nil)
  case deleted
  case removed(revision: String, previousRevision: String? = nil)
  case rejected(String, status: Int? = nil)
  case publication(
    intentId: UUID, status: PublicationStatus, revision: String, confirmedPublished: Bool = false,
    previousRevision: String? = nil)
}

public protocol SyncRemote: Sendable {
  func supportsNativeSync() async throws -> Bool
  func receipt(operationId: UUID) async throws -> OperationReceipt
  func uploadReceipt(voyageId: UUID, key: UUID) async throws -> OperationReceipt
  func publicationStatus(voyageId: UUID, intentId: UUID) async throws -> OperationReceipt
  func isPublished(voyageId: UUID) async throws -> Bool
  func submit(_ request: MutationRequest, operationId: UUID) async throws -> OperationReceipt
}

extension SyncRemote {
  public func isPublished(voyageId: UUID) async throws -> Bool {
    throw WorkspaceError.unsupportedProtocol
  }
  public func uploadReceipt(voyageId: UUID, key: UUID) async throws -> OperationReceipt {
    try await receipt(operationId: key)
  }
  public func publicationStatus(voyageId: UUID, intentId: UUID) async throws -> OperationReceipt {
    .inProgress
  }
}

public protocol UploadSubmitting: Sendable {
  func submit(operation: OutboxOperation, request: MutationRequest, workspace: Workspace)
    async throws
  func cancel(operation: OutboxOperation) async
  func cancel(accountId: UUID) async
  func cancel(accountId: UUID, generation: Int) async
}

extension UploadSubmitting {
  public func cancel(operation: OutboxOperation) async {}
  public func cancel(accountId: UUID, generation: Int) async { await cancel(accountId: accountId) }
}

/// A bounded pass owns sync independently of the lifetime of any observing view.
public actor SyncCoordinator {
  private let store: any WorkspaceStore
  private let remote: any SyncRemote
  private let uploads: (any UploadSubmitting)?
  private let randomUnit: @Sendable () -> Double
  private var running: Set<UUID> = []

  public init(
    store: any WorkspaceStore, remote: any SyncRemote, uploads: (any UploadSubmitting)? = nil,
    randomUnit: @escaping @Sendable () -> Double = { Double.random(in: 0...1) }
  ) {
    self.store = store
    self.remote = remote
    self.uploads = uploads
    self.randomUnit = randomUnit
  }

  public func run(accountId: UUID, network: NetworkState, now: Date = Date()) async throws {
    guard !running.contains(accountId) else { return }
    running.insert(accountId)
    defer { running.remove(accountId) }
    let initial = try await store.read(accountId: accountId)
    for operation in initial.operations {
      if case .upload(let photoId, _) = operation.kind,
        initial.photos.contains(where: { $0.id == photoId && $0.deleted })
          || initial.uploadPolicy(for: photoId).paused
      {
        await uploads?.cancel(operation: operation)
      }
    }
    guard initial.account.access == .unlocked, !initial.policy.paused, network != .offline else {
      return
    }
    guard try await remote.supportsNativeSync() else { throw WorkspaceError.unsupportedProtocol }
    for intent in initial.publicationIntents where intent.isArmed {
      guard let serverIntent = intent.serverId,
        let voyage = initial.voyages.first(where: { $0.id == intent.voyageId }),
        let serverVoyage = voyage.serverId,
        let operation = initial.operations.first(where: { operation in
          if case .publish(let id) = operation.kind { return id == intent.id }
          return false
        })
      else { continue }
      guard operation.nextAttempt <= now,
        ![.attention, .conflict, .canceled].contains(operation.state)
      else { continue }
      do {
        let receipt = try await remote.publicationStatus(
          voyageId: serverVoyage, intentId: serverIntent)
        if case .publication = receipt {
          try await apply(receipt, operation: operation, generation: initial.account.generation)
        }
      } catch {
        try await recordFailure(
          error, operation: operation, generation: initial.account.generation, now: now,
          allowApplied: true)
      }
    }
    var visited: Set<UUID> = []
    // Receipts may describe our own intervening writes. Resolve them before preparing
    // any new If-Match request, even when upload bytes are waiting for Wi-Fi.
    // Settled ledger entries must never hide pending receipts, including those beyond one write batch.
    for operation in initial.operations
    where operation.request != nil && [.submitted, .uncertain, .waiting].contains(operation.state)
      && operation.nextAttempt <= now
    {
      let current = try await store.read(accountId: accountId)
      guard current.account.access == .unlocked,
        current.account.generation == initial.account.generation, !current.policy.paused,
        !Task.isCancelled
      else { return }
      do {
        let receipt: OperationReceipt
        if case .upload(let photoId, _) = operation.kind,
          let photo = current.photos.first(where: { $0.id == photoId }),
          let voyage = current.voyages.first(where: { $0.id == operation.voyageId }),
          let remoteId = voyage.serverId
        {
          receipt = try await remote.uploadReceipt(voyageId: remoteId, key: photo.uploadKey)
        } else {
          receipt = try await remote.receipt(operationId: operation.id)
        }
        if receipt != .notSeen {
          visited.insert(operation.id)
          try await apply(receipt, operation: operation, generation: initial.account.generation)
        }
      } catch {
        visited.insert(operation.id)
        try await recordFailure(
          error, operation: operation,
          generation: initial.account.generation, now: now)
      }
    }
    var lastVoyage: UUID?
    for _ in 0..<100 {
      guard !Task.isCancelled else { return }
      let workspace = try await store.read(accountId: accountId)
      guard workspace.account.access == .unlocked,
        workspace.account.generation == initial.account.generation, !workspace.policy.paused
      else { return }
      let candidates = workspace.operations.filter { operation in
        guard !visited.contains(operation.id),
          [.waiting, .submitted, .uncertain].contains(operation.state), operation.nextAttempt <= now
        else { return false }
        guard
          operation.dependencies.allSatisfy({ id in
            workspace.operations.contains {
              $0.id == id && [.applied, .canceled].contains($0.state)
            }
          })
        else { return false }
        if operation.kind.priority != 0
          && workspace.operations.contains(where: {
            $0.scope == operation.scope && [.conflict, .attention].contains($0.state)
          })
        {
          return false
        }
        if case .order(let ids) = operation.kind {
          guard
            ids.allSatisfy({ id in
              workspace.photos.contains { $0.id == id && !$0.deleted && $0.serverId != nil }
            })
          else { return false }
        }
        if case .upload(let photoId, _) = operation.kind {
          guard workspace.photos.contains(where: { $0.id == photoId && !$0.deleted }) else {
            return false
          }
          if operation.request != nil { return true }
          return workspace.account.profile.displayName != nil && uploads != nil
            && workspace.uploadPolicy(for: photoId).waitReason(
              network: network, authenticated: true) == nil
        }
        return true
      }.sorted {
        if $0.kind.priority != $1.kind.priority { return $0.kind.priority < $1.kind.priority }
        if ($0.voyageId == lastVoyage) != ($1.voyageId == lastVoyage) {
          return $0.voyageId != lastVoyage
        }
        return $0.id.uuidString < $1.id.uuidString
      }
      guard var operation = candidates.first else { break }
      visited.insert(operation.id)
      lastVoyage = operation.voyageId
      do {
        if case .deleteVoyage = operation.kind,
          workspace.voyages.first(where: { $0.id == operation.voyageId })?.serverId == nil
        {
          try await apply(.deleted, operation: operation, generation: initial.account.generation)
          continue
        }
        if case .upload(let photoId, _) = operation.kind,
          uploads == nil || workspace.account.profile.displayName == nil
            || workspace.uploadPolicy(for: photoId).waitReason(
              network: network, authenticated: true) != nil
        {
          continue
        }
        if operation.request == nil {
          operation.request = try RequestFactory.prepare(operation, workspace: workspace, now: now)
          let prepared = operation
          try await store.update(accountId: accountId) { state in
            guard state.account.generation == initial.account.generation,
              state.account.access == .unlocked,
              let index = state.operations.firstIndex(where: { $0.id == prepared.id })
            else { throw WorkspaceError.locked }
            guard state.operations[index].request == nil,
              state.operations[index].kind == prepared.kind,
              [.waiting, .submitted, .uncertain].contains(state.operations[index].state)
            else { return }
            state.operations[index].request = prepared.request
            state.operations[index].state = .submitted
          }
          let checkpoint = try await store.read(accountId: accountId)
          guard checkpoint.account.access == .unlocked,
            checkpoint.account.generation == initial.account.generation,
            checkpoint.operations.contains(where: {
              $0.id == prepared.id && $0.kind == prepared.kind
                && $0.request == prepared.request && $0.state == .submitted
            })
          else { continue }
        }
        if case .upload = operation.kind {
          try await uploads?.submit(
            operation: operation, request: operation.request!, workspace: workspace)
        } else {
          let receipt = try await remote.submit(operation.request!, operationId: operation.id)
          guard receipt != .notSeen else { throw WorkspaceError.malformedReceipt }
          try await apply(receipt, operation: operation, generation: initial.account.generation)
        }
      } catch {
        try await recordFailure(
          error, operation: operation, generation: initial.account.generation, now: now)
      }
    }
  }

  public func cancelTransfers(accountId: UUID) async { await uploads?.cancel(accountId: accountId) }

  private static func settleRemovedUpload(_ photoId: UUID, state: inout Workspace) {
    for index in state.operations.indices {
      if case .upload(let id, _) = state.operations[index].kind, id == photoId,
        state.operations[index].state != .applied
      {
        state.operations[index].state = .canceled
        state.operations[index].lastError = nil
      }
    }
  }

  private func apply(_ receipt: OperationReceipt, operation: OutboxOperation, generation: Int)
    async throws
  {
    if case .applied = receipt {
      switch operation.kind {
      case .publish, .cancelPublication: throw WorkspaceError.malformedReceipt
      default: break
      }
    }
    let published: Bool?
    if case .unpublish = operation.kind, case .applied(let id, _, _) = receipt {
      published = try await remote.isPublished(voyageId: id)
    } else {
      published = nil
    }
    try await store.update(accountId: operation.accountId) { state in
      guard state.account.generation == generation, state.account.access == .unlocked,
        let index = state.operations.firstIndex(where: { $0.id == operation.id })
      else { return }
      switch receipt {
      case .notSeen: return
      case .inProgress:
        state.operations[index].state = .uncertain
        return
      case .rejected(let message, let status):
        state.operations[index].receiptRejected = true
        let disposition = status.map(ResponseDisposition.classify)
        state.operations[index].state =
          disposition == .conflict || disposition == .destinationMissing ? .conflict : .attention
        state.operations[index].lastError = message
        if case .publish(let id) = operation.kind,
          let intent = state.publicationIntents.firstIndex(where: { $0.id == id }),
          state.publicationIntents[intent].serverId == nil,
          state.publicationIntents[intent].cancellationPending
        {
          state.publicationIntents[intent].status = .canceled
          state.publicationIntents[intent].cancellationPending = false
          LocalRepository.cancelLocalPublicationOperations(&state, intentIds: [id])
        }
        return
      case .removed(let revision, let previous):
        Self.advanceRevision(revision, previous: previous, operation: operation, state: &state)
        fallthrough
      case .deleted:
        switch operation.kind {
        case .upload(let photoId, _)
        where state.photos.contains(where: { $0.id == photoId && $0.deleted }):
          break
        case .create, .upload, .edit, .order, .publish, .unpublish:
          if case .publish(let id) = operation.kind,
            let intent = state.publicationIntents.firstIndex(where: { $0.id == id })
          {
            state.publicationIntents[intent].status = .needsReview
          }
          state.operations[index].state = .conflict
          state.operations[index].lastError =
            "The destination was deleted or its upload was canceled. Your local work has been kept."
          return
        case .removePhoto(let photoId, _):
          if let photo = state.photos.firstIndex(where: { $0.id == photoId }) {
            state.photos[photo].transfer = .removed
            Self.settleRemovedUpload(photoId, state: &state)
          }
        case .cancelPublication(let id):
          if let intent = state.publicationIntents.firstIndex(where: { $0.id == id }) {
            state.publicationIntents[intent].status = .canceled
            state.publicationIntents[intent].cancellationPending = false
          }
        default: break
        }
        state.operations[index].state = .applied
      case .applied(let id, let revision, let previous):
        state.operations[index].state = .applied
        Self.advanceRevision(revision, previous: previous, operation: operation, state: &state)
        if let voyage = state.voyages.firstIndex(where: { $0.id == operation.voyageId }) {
          switch operation.kind {
          case .create: state.voyages[voyage].serverId = id
          case .unpublish:
            state.voyages[voyage].confirmedPublished =
              published ?? state.voyages[voyage].confirmedPublished
          case .upload(let photoId, _):
            if let photo = state.photos.firstIndex(where: { $0.id == photoId }) {
              state.photos[photo].serverId = id
              state.photos[photo].cellularException = nil
              state.photos[photo].transfer = state.photos[photo].deleted ? .removing : .accepted
            }
          case .removePhoto(let photoId, _):
            if let photo = state.photos.firstIndex(where: { $0.id == photoId }) {
              state.photos[photo].transfer = .removed
              Self.settleRemovedUpload(photoId, state: &state)
            }
          default: break
          }

        }
        if case .block(let userId, let blocked) = operation.kind, !blocked {
          state.blockedAccounts.remove(userId)
        }
      case .publication(
        let intentId, let status, let revision, let confirmedPublished, let previous):
        state.operations[index].state = .applied
        let localIntentId: UUID
        switch operation.kind {
        case .publish(let id), .cancelPublication(let id): localIntentId = id
        default: throw WorkspaceError.malformedReceipt
        }
        if let intent = state.publicationIntents.firstIndex(where: { $0.id == localIntentId }) {
          state.publicationIntents[intent].serverId = intentId
          state.publicationIntents[intent].status = status
          if [.published, .canceled, .expired, .needsReview, .failed].contains(status) {
            state.publicationIntents[intent].cancellationPending = false
          }
        }
        if let voyage = state.voyages.firstIndex(where: { $0.id == operation.voyageId }) {
          if let previous {
            Self.advanceRevision(revision, previous: previous, operation: operation, state: &state)
          }
          state.voyages[voyage].confirmedPublished = confirmedPublished
        }
      }
      if state.operations[index].completedAt == nil { state.operations[index].completedAt = Date() }
      state.operations[index].attempts = 0
      state.operations[index].nextAttempt = .distantPast
      state.operations[index].lastError = nil
    }
  }

  /// Only a connected chain of acknowledged writes can advance the loaded snapshot.
  /// Disconnected edges remain durable until earlier receipts arrive or the owner refreshes.
  private static func advanceRevision(
    _ revision: String, previous: String?, operation: OutboxOperation, state: inout Workspace
  ) {
    guard let voyage = state.voyages.firstIndex(where: { $0.id == operation.voyageId }),
      let receipt = state.operations.firstIndex(where: { $0.id == operation.id })
    else { return }
    state.operations[receipt].receiptPreviousRevision = previous ?? operation.request?.revision
    state.operations[receipt].receiptRevision = revision
    if case .create(let fields) = operation.kind, state.voyages[voyage].serverId == nil {
      state.voyages[voyage].revision = revision
      state.voyages[voyage].serverFields = fields
    }
    var consumed: Set<UUID> = []
    while let edge = state.operations.first(where: {
      $0.voyageId == operation.voyageId && !consumed.contains($0.id)
        && $0.receiptPreviousRevision != nil
        && $0.receiptPreviousRevision == state.voyages[voyage].revision
        && $0.receiptRevision != nil
    }) {
      consumed.insert(edge.id)
      let previous = state.voyages[voyage].revision
      state.voyages[voyage].revision = edge.receiptRevision
      switch edge.kind {
      case .create(let fields), .edit(let fields): state.voyages[voyage].serverFields = fields
      default: break
      }
      for successor in state.operations.indices
      where state.operations[successor].voyageId == operation.voyageId
        && state.operations[successor].request == nil
        && state.operations[successor].expectedRevision == previous
      {
        state.operations[successor].expectedRevision = edge.receiptRevision
      }
    }
  }

  private func recordFailure(
    _ error: any Error, operation: OutboxOperation, generation: Int, now: Date,
    allowApplied: Bool = false
  ) async throws {
    let failure = SynchronizationFailure(error)
    let jitter = randomUnit()
    try await store.update(accountId: operation.accountId) { state in
      failure.apply(
        to: &state, operationId: operation.id, generation: generation, now: now, randomUnit: jitter,
        allowApplied: allowApplied)
    }
    if failure.disposition == .signIn {
      await uploads?.cancel(accountId: operation.accountId, generation: generation)
    }
  }
}
