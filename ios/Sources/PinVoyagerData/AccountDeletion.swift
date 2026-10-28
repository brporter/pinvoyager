import Foundation
import PinVoyagerDomain

public struct DeletionProof: Decodable, Sendable {
  public let proofToken: String
  public let accountId: UUID
  public let provider: String
  public let purpose: String
  public let expiresAt: String
}
public enum AccountDeletionStatus: String, Codable, Sendable {
  case preparing, prepared, confirming, deleting, complete, expired
}
public struct PendingAccountDeletion: Codable, Equatable, Identifiable, Sendable {
  public let id: UUID
  public let accountId: UUID
  public let statusCapability: String
  public var proofToken: String?
  public var appleProofToken: String?
  public var status: AccountDeletionStatus
  public var expiresAt: Date
}
public protocol DeletionRecoveryStore: Sendable {
  func read(accountId: UUID) async throws -> PendingAccountDeletion?
  func save(_ record: PendingAccountDeletion) async throws
  func all() async throws -> [PendingAccountDeletion]
  func remove(accountId: UUID) async throws
}
public struct KeychainDeletionRecoveryStore: DeletionRecoveryStore {
  private let records: KeychainRecords<PendingAccountDeletion>
  public init(
    service: String = "com.pinvoyager.native.deletions",
    backend: any KeychainBackend = SystemKeychainBackend()
  ) {
    records = KeychainRecords(service: service, backend: backend)
  }
  public func read(accountId: UUID) async throws -> PendingAccountDeletion? {
    try await records.read(accountId: accountId)
  }
  public func save(_ record: PendingAccountDeletion) async throws {
    try await records.save(record, accountId: record.accountId)
  }
  public func all() async throws -> [PendingAccountDeletion] { try await records.all() }
  public func remove(accountId: UUID) async throws {
    try await records.remove(accountId: accountId)
  }
}

/// Persist the independent recovery credential before any request can remove a session.
public actor AccountDeletionService {
  private let client: APIClient
  public let recovery: any DeletionRecoveryStore
  public init(client: APIClient, recovery: any DeletionRecoveryStore) {
    self.client = client
    self.recovery = recovery
  }
  public func prepare(accountId: UUID, proofToken: String, appleProofToken: String? = nil)
    async throws -> PendingAccountDeletion
  {
    if let previous = try await recovery.read(accountId: accountId),
      [.confirming, .deleting].contains(previous.status)
        || (previous.status == .preparing && previous.expiresAt > Date())
    {
      throw ValidationError(
        field: "deletion",
        message:
          "Check the pending preparation or deletion outcome before preparing another request.")
    }
    let record = PendingAccountDeletion(
      id: UUID(), accountId: accountId, statusCapability: try NativeAuthentication.randomToken(),
      proofToken: proofToken, appleProofToken: appleProofToken, status: .preparing,
      expiresAt: Date().addingTimeInterval(600))
    try await recovery.save(record)
    return try await retryPreparation(accountId: accountId)
  }
  public func retryPreparation(accountId: UUID) async throws -> PendingAccountDeletion {
    guard var record = try await recovery.read(accountId: accountId), record.status == .preparing,
      record.expiresAt > Date(), let proof = record.proofToken
    else { throw AuthenticationError.invalidAttempt }
    struct Request: Encodable {
      let proofToken: String
      let appleProofToken: String?
      let statusCapability: String
    }
    struct Reply: Decodable, Sendable {
      let id: UUID
      let status: AccountDeletionStatus
      let expiresAt: String
    }
    let reply: Reply = try await client.send(
      method: "POST", path: "/api/me/deletion-requests",
      body: APIJSON.encode(
        Request(
          proofToken: proof, appleProofToken: record.appleProofToken,
          statusCapability: record.statusCapability)), accountId: accountId,
      headers: ["Idempotency-Key": record.id.uuidString])
    guard reply.id == record.id, reply.status == .prepared,
      let expires = APIJSON.date(reply.expiresAt)
    else { throw WorkspaceError.malformedReceipt }
    record.status = .prepared
    record.expiresAt = expires
    record.proofToken = nil
    record.appleProofToken = nil
    try await recovery.save(record)
    return record
  }
  public func confirm(accountId: UUID) async throws -> PendingAccountDeletion {
    guard var record = try await recovery.read(accountId: accountId), record.status == .prepared,
      record.expiresAt > Date()
    else { throw AuthenticationError.invalidAttempt }
    record.status = .confirming
    try await recovery.save(record)
    struct Reply: Decodable, Sendable {
      let id: UUID
      let status: AccountDeletionStatus
    }
    let reply: Reply = try await client.send(
      method: "POST", path: "/api/me/deletion-requests/\(record.id)/confirm", accountId: accountId)
    guard reply.id == record.id, [.deleting, .complete].contains(reply.status) else {
      throw WorkspaceError.malformedReceipt
    }
    record.status = reply.status
    try await recovery.save(record)
    return record
  }
  public func refresh(accountId: UUID) async throws -> PendingAccountDeletion {
    guard var record = try await recovery.read(accountId: accountId) else {
      throw WorkspaceError.missing
    }
    struct Reply: Decodable, Sendable { let status: AccountDeletionStatus }
    let reply: Reply = try await client.send(
      method: "GET", path: "/api/account-deletions/\(record.id)",
      headers: ["Authorization": "Deletion \(record.statusCapability)"])
    guard [.prepared, .expired, .deleting, .complete].contains(reply.status) else {
      throw WorkspaceError.malformedReceipt
    }
    record.status = reply.status
    if reply.status != .prepared {
      record.proofToken = nil
      record.appleProofToken = nil
    }
    try await recovery.save(record)
    return record
  }
}
