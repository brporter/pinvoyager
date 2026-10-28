import Foundation
import PinVoyagerDomain

public struct TransferDescriptor: Equatable, Sendable {
  public let accountId: UUID
  public let generation: Int
  public let operationId: UUID
  public let cellularException: Bool
  public init(accountId: UUID, generation: Int, operationId: UUID, cellularException: Bool = false)
  {
    self.accountId = accountId
    self.generation = generation
    self.operationId = operationId
    self.cellularException = cellularException
  }
  public var rawValue: String {
    "\(accountId):\(generation):\(operationId)" + (cellularException ? ":cellular" : "")
  }
  public init?(rawValue: String) {
    let parts = rawValue.split(separator: ":", omittingEmptySubsequences: false)
    guard parts.count == 3 || (parts.count == 4 && parts[3] == "cellular"),
      let accountId = UUID(uuidString: String(parts[0])),
      let generation = Int(parts[1]), generation >= 0,
      let operationId = UUID(uuidString: String(parts[2]))
    else { return nil }
    self.init(
      accountId: accountId, generation: generation, operationId: operationId,
      cellularException: parts.count == 4)
  }
  public var sessionIdentifier: String {
    TransferSession(
      accountId: accountId, generation: generation, cellularException: cellularException
    ).identifier
  }
}

public struct TransferCompletion: Sendable {
  public let descriptor: TransferDescriptor
  public let taskId: Int?
  public let response: HTTPResponse?
  public let transportError: URLError.Code?
  public init(
    descriptor: TransferDescriptor, taskId: Int? = nil, response: HTTPResponse? = nil,
    transportError: URLError.Code? = nil
  ) {
    self.descriptor = descriptor
    self.taskId = taskId
    self.response = response
    self.transportError = transportError
  }
}

public struct SystemTransfer: Equatable, Sendable {
  public let id: Int
  public let descriptor: TransferDescriptor
  public init(id: Int, descriptor: TransferDescriptor) {
    self.id = id
    self.descriptor = descriptor
  }
}

public protocol TransferDriver: Sendable {
  func configure(sessionIdentifier: String, policy: SyncPolicy) async
  func pendingCompletions(sessionIdentifier: String) async throws -> [TransferCompletion]
  func discardCompletion(_ completion: TransferCompletion) async
  func tasks(sessionIdentifier: String) async -> [SystemTransfer]
  func createSuspended(
    sessionIdentifier: String, request: URLRequest, body: URL, descriptor: TransferDescriptor,
    policy: SyncPolicy
  ) async throws -> SystemTransfer
  func resume(_ task: SystemTransfer, sessionIdentifier: String) async
  func cancel(sessionIdentifier: String) async
  func cancel(_ task: SystemTransfer, sessionIdentifier: String) async
}

extension TransferDriver {
  public func pendingCompletions(sessionIdentifier: String) async throws -> [TransferCompletion] {
    []
  }
  public func discardCompletion(_ completion: TransferCompletion) async {}
  public func configure(sessionIdentifier: String, policy: SyncPolicy) async {}
}

public actor BackgroundUploadService: UploadSubmitting {
  private let store: any WorkspaceStore
  private let vault: FileVault
  private let credentials: any CredentialStore
  private let driver: any TransferDriver
  private let origin: TrustedOrigin
  private let maxBytes: Int64
  public init(
    store: any WorkspaceStore, vault: FileVault, credentials: any CredentialStore,
    driver: any TransferDriver, origin: TrustedOrigin, maxBytes: Int64
  ) {
    self.store = store
    self.vault = vault
    self.credentials = credentials
    self.driver = driver
    self.origin = origin
    self.maxBytes = maxBytes
  }

  public func submit(operation: OutboxOperation, request: MutationRequest, workspace: Workspace)
    async throws
  {
    guard case .upload(let photoId, _) = operation.kind,
      let photo = workspace.photos.first(where: { $0.id == photoId && !$0.deleted }),
      let source = photo.source,
      let credential = try await credentials.read(accountId: operation.accountId),
      !credential.revocationOnly,
      workspace.account.access == .unlocked
    else { throw AuthenticationError.signInRequired }
    let generation = workspace.account.generation
    let descriptor = TransferDescriptor(
      accountId: operation.accountId, generation: generation, operationId: operation.id,
      cellularException: photo.cellularException == true)
    await driver.configure(
      sessionIdentifier: descriptor.sessionIdentifier, policy: workspace.uploadPolicy(for: photoId))
    let existing = await driver.tasks(sessionIdentifier: descriptor.sessionIdentifier)
    if let task = existing.first(where: { $0.descriptor == descriptor }) {
      try await activate(task, photoId: photoId, descriptor: descriptor)
      return
    }
    let bodyPath = "bodies/\(operation.id).multipart"
    let destination = try await vault.url(bodyPath, accountId: operation.accountId)
    let boundary = "PinVoyager-\(operation.id)"
    let multipart: MultipartBody
    if FileManager.default.fileExists(atPath: destination.path) {
      let size = Int64(try destination.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
      guard size <= maxBytes else { throw FileVaultError.tooLarge }
      multipart = MultipartBody(
        url: destination, byteCount: size, contentType: "multipart/form-data; boundary=\(boundary)")
    } else {
      let input = try await vault.url(source.relativePath, accountId: operation.accountId)
      multipart = try MultipartBody.write(
        source: input, destination: destination, mediaType: source.mediaType, boundary: boundary,
        maxBytes: maxBytes)
    }
    let record = BackgroundTransfer(
      operationId: operation.id, generation: generation,
      sessionIdentifier: descriptor.sessionIdentifier, bodyPath: bodyPath,
      byteCount: multipart.byteCount)
    try await store.update(accountId: operation.accountId) { state in
      guard state.account.generation == generation, state.account.access == .unlocked,
        !state.uploadPolicy(for: photoId).paused,
        state.photos.contains(where: { $0.id == photoId && !$0.deleted })
      else { throw WorkspaceError.locked }
      if let index = state.transfers.firstIndex(where: { $0.operationId == operation.id }) {
        state.transfers[index] = record
      } else {
        state.transfers.append(record)
      }
    }
    var urlRequest = URLRequest(url: try origin.resolve(request.path))
    urlRequest.httpMethod = request.method
    urlRequest.httpShouldHandleCookies = false
    urlRequest.setValue("Bearer \(credential.token)", forHTTPHeaderField: "Authorization")
    urlRequest.setValue(multipart.contentType, forHTTPHeaderField: "Content-Type")
    for (name, value) in request.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    let task = try await driver.createSuspended(
      sessionIdentifier: descriptor.sessionIdentifier, request: urlRequest, body: multipart.url,
      descriptor: descriptor, policy: workspace.uploadPolicy(for: photoId))
    try await activate(task, photoId: photoId, descriptor: descriptor)
  }

  private func activate(_ task: SystemTransfer, photoId: UUID, descriptor: TransferDescriptor)
    async throws
  {
    do {
      try await persistTask(task, photoId: photoId, descriptor: descriptor)
      await driver.resume(task, sessionIdentifier: descriptor.sessionIdentifier)
    } catch {
      await driver.cancel(task, sessionIdentifier: descriptor.sessionIdentifier)
      throw error
    }
  }

  private func persistTask(_ task: SystemTransfer, photoId: UUID, descriptor: TransferDescriptor)
    async throws
  {
    try await store.update(accountId: descriptor.accountId) { state in
      guard state.account.generation == descriptor.generation, state.account.access == .unlocked,
        !state.uploadPolicy(for: photoId).paused,
        state.photos.contains(where: { $0.id == photoId && !$0.deleted })
      else { throw WorkspaceError.locked }
      if let index = state.transfers.firstIndex(where: { $0.operationId == descriptor.operationId })
      {
        state.transfers[index].taskId = task.id
      }
      if let index = state.photos.firstIndex(where: { $0.id == photoId && !$0.deleted }) {
        state.photos[index].transfer = .scheduled
      }
    }
  }

  public func completed(_ descriptor: TransferDescriptor) async throws {
    try await BackgroundTransferRecovery(store: store, credentials: credentials, driver: driver)
      .completed(descriptor)
  }

  public func cancel(operation: OutboxOperation) async {
    await BackgroundTransferRecovery(store: store, credentials: credentials, driver: driver)
      .cancel(operation: operation)
  }

  public func cancel(accountId: UUID, generation: Int) async {
    await BackgroundTransferRecovery(store: store, credentials: credentials, driver: driver)
      .cancel(accountId: accountId, generation: generation)
  }

  public func cancel(accountId: UUID) async {
    await BackgroundTransferRecovery(store: store, credentials: credentials, driver: driver).cancel(
      accountId: accountId)
  }
}
