import Foundation
import PinVoyagerDomain

public struct TransferSession: Equatable, Sendable {
  public let accountId: UUID
  public let generation: Int
  public let cellularException: Bool
  public init(accountId: UUID, generation: Int, cellularException: Bool = false) {
    self.accountId = accountId
    self.generation = generation
    self.cellularException = cellularException
  }
  public var identifier: String {
    "com.pinvoyager.upload.\(accountId.uuidString.lowercased()).\(generation)"
      + (cellularException ? ".cellular" : "")
  }
  public init?(identifier: String) {
    let parts = identifier.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count == 5 || (parts.count == 6 && parts[5] == "cellular"),
      parts.prefix(3) == ["com", "pinvoyager", "upload"],
      let accountId = UUID(uuidString: String(parts[3])), let generation = Int(parts[4]),
      generation >= 0
    else { return nil }
    self.init(accountId: accountId, generation: generation, cellularException: parts.count == 6)
  }
}

/// Restore policies and task identities before resuming work or acknowledging system wake events.
public struct BackgroundTransferRecovery: Sendable {
  private let store: any WorkspaceStore
  private let credentials: any CredentialStore
  private let driver: any TransferDriver
  public init(
    store: any WorkspaceStore, credentials: any CredentialStore, driver: any TransferDriver
  ) {
    self.store = store
    self.credentials = credentials
    self.driver = driver
  }
  public func restore() async throws {
    for accountId in try await store.accountIds() {
      let state = try await store.read(accountId: accountId)
      let credential = try await credentials.read(accountId: accountId)
      let identifiers = Set(state.transfers.map(\.sessionIdentifier))
      for identifier in identifiers {
        guard let session = TransferSession(identifier: identifier), session.accountId == accountId
        else { continue }
        await driver.configure(
          sessionIdentifier: identifier,
          policy: state.policy.forUpload(cellularException: session.cellularException))
        for completion in try await driver.pendingCompletions(sessionIdentifier: identifier) {
          try await completed(completion)
        }
        let state = try await store.read(accountId: accountId)
        guard state.account.access == .unlocked, session.generation == state.account.generation,
          !state.policy.paused, credential != nil, credential?.revocationOnly == false
        else {
          await driver.cancel(sessionIdentifier: identifier)
          continue
        }
        for task in await driver.tasks(sessionIdentifier: identifier) {
          guard task.descriptor.accountId == accountId,
            task.descriptor.generation == session.generation,
            task.descriptor.cellularException == session.cellularException,
            let operation = state.operations.first(where: { $0.id == task.descriptor.operationId }),
            operation.request != nil, [.waiting, .submitted, .uncertain].contains(operation.state),
            case .upload(let photoId, _) = operation.kind,
            !state.uploadPolicy(for: photoId).paused,
            state.photos.contains(where: {
              $0.id == photoId && !$0.deleted
                && (!session.cellularException || $0.cellularException == true)
            }),
            state.transfers.contains(where: { $0.operationId == operation.id })
          else {
            await driver.cancel(task, sessionIdentifier: identifier)
            continue
          }
          do {
            try await store.update(accountId: accountId) { current in
              guard current.account.access == .unlocked,
                current.account.generation == session.generation,
                !current.uploadPolicy(for: photoId).paused,
                current.photos.contains(where: {
                  $0.id == photoId && !$0.deleted
                    && (!session.cellularException || $0.cellularException == true)
                }),
                let index = current.transfers.firstIndex(where: { $0.operationId == operation.id })
              else { throw WorkspaceError.locked }
              current.transfers[index].taskId = task.id
            }
            await driver.resume(task, sessionIdentifier: identifier)
          } catch WorkspaceError.locked {
            await driver.cancel(task, sessionIdentifier: identifier)
          }
        }
      }
    }
  }
  public func completed(_ descriptor: TransferDescriptor) async throws {
    try await completed(TransferCompletion(descriptor: descriptor))
  }

  public func completed(
    _ completion: TransferCompletion, now: Date = Date(),
    randomUnit: Double = Double.random(in: 0...1)
  ) async throws {
    let descriptor = completion.descriptor
    let failure: SynchronizationFailure?
    if let code = completion.transportError, code != .cancelled {
      failure = SynchronizationFailure(URLError(code))
    } else if let response = completion.response, !(200..<300).contains(response.status),
      completion.transportError == nil
    {
      failure = SynchronizationFailure(APIError(response: response, now: now))
    } else {
      failure = nil
    }
    try await store.update(accountId: descriptor.accountId) { state in
      guard state.account.access == .unlocked, state.account.generation == descriptor.generation,
        let transfer = state.transfers.firstIndex(where: {
          $0.operationId == descriptor.operationId
        }),
        !state.transfers[transfer].completed,
        completion.taskId == nil || state.transfers[transfer].taskId == completion.taskId,
        let operation = state.operations.firstIndex(where: { $0.id == descriptor.operationId })
      else { return }
      state.transfers[transfer].completed = true
      guard ![.applied, .canceled].contains(state.operations[operation].state) else { return }
      // A successful HTTP response still needs its authoritative server receipt.
      state.operations[operation].state = .uncertain
      failure?.apply(
        to: &state, operationId: descriptor.operationId, generation: descriptor.generation,
        now: now, randomUnit: randomUnit)
    }
    if failure?.disposition == .signIn {
      await cancel(accountId: descriptor.accountId, generation: descriptor.generation)
    }
    await driver.discardCompletion(completion)
  }
  public func cancel(operation: OutboxOperation) async {
    guard let state = try? await store.read(accountId: operation.accountId) else { return }
    for identifier in Set(
      state.transfers.filter { $0.operationId == operation.id }.map(\.sessionIdentifier))
    {
      guard TransferSession(identifier: identifier)?.accountId == operation.accountId else {
        continue
      }
      for task in await driver.tasks(sessionIdentifier: identifier)
      where task.descriptor.operationId == operation.id {
        await driver.cancel(task, sessionIdentifier: identifier)
      }
    }
  }

  public func cancelIneligible(accountId: UUID) async {
    guard let state = try? await store.read(accountId: accountId) else { return }
    for operation in state.operations {
      if case .upload(let photoId, _) = operation.kind,
        state.photos.contains(where: { $0.id == photoId && $0.deleted })
          || state.uploadPolicy(for: photoId).paused
      {
        await cancel(operation: operation)
      }
    }
  }

  public func cancel(accountId: UUID, generation: Int? = nil) async {
    guard let state = try? await store.read(accountId: accountId) else { return }
    for identifier in Set(state.transfers.map(\.sessionIdentifier))
    where TransferSession(identifier: identifier)?.accountId == accountId
      && (generation == nil || TransferSession(identifier: identifier)?.generation == generation)
    {
      await driver.cancel(sessionIdentifier: identifier)
    }
  }
}
