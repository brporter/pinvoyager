import Foundation
import Security

public struct SessionCredential: Codable, Equatable, Sendable {
  public let accountId: UUID
  public let token: String
  public var revocationOnly = false
  public var idleDeadline: String?
  public var absoluteDeadline: String?
  public init(accountId: UUID, token: String) {
    self.accountId = accountId
    self.token = token
  }
}

public protocol CredentialStore: Sendable {
  func read(accountId: UUID) async throws -> SessionCredential?
  func save(_ credential: SessionCredential) async throws
  func remove(accountId: UUID) async throws
  func all() async throws -> [SessionCredential]
}

public struct KeychainError: Error, Sendable { public let status: OSStatus }

public protocol KeychainBackend: Sendable {
  func copy(_ query: CFDictionary, result: UnsafeMutablePointer<CFTypeRef?>) -> OSStatus
  func update(_ query: CFDictionary, attributes: CFDictionary) -> OSStatus
  func add(_ attributes: CFDictionary) -> OSStatus
  func delete(_ query: CFDictionary) -> OSStatus
}

public struct SystemKeychainBackend: KeychainBackend {
  public init() {}
  public func copy(_ query: CFDictionary, result: UnsafeMutablePointer<CFTypeRef?>) -> OSStatus {
    SecItemCopyMatching(query, result)
  }
  public func update(_ query: CFDictionary, attributes: CFDictionary) -> OSStatus {
    SecItemUpdate(query, attributes)
  }
  public func add(_ attributes: CFDictionary) -> OSStatus { SecItemAdd(attributes, nil) }
  public func delete(_ query: CFDictionary) -> OSStatus { SecItemDelete(query) }
}

public struct KeychainCredentialStore: CredentialStore {
  private let records: KeychainRecords<SessionCredential>
  public init(
    service: String = "com.pinvoyager.native.sessions",
    backend: any KeychainBackend = SystemKeychainBackend()
  ) {
    records = KeychainRecords(service: service, backend: backend)
  }
  public func read(accountId: UUID) async throws -> SessionCredential? {
    try await records.read(accountId: accountId)
  }
  public func save(_ credential: SessionCredential) async throws {
    try await records.save(credential, accountId: credential.accountId)
  }
  public func remove(accountId: UUID) async throws {
    try await records.remove(accountId: accountId)
  }
  public func all() async throws -> [SessionCredential] { try await records.all() }
}

actor KeychainRecords<Record: Codable & Sendable> {

  private let service: String
  private let backend: any KeychainBackend
  public init(
    service: String = "com.pinvoyager.native.sessions",
    backend: any KeychainBackend = SystemKeychainBackend()
  ) {
    self.service = service
    self.backend = backend
  }
  private func query(_ accountId: UUID? = nil) -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
    ]
    if let accountId { query[kSecAttrAccount as String] = accountId.uuidString }
    return query
  }
  public func read(accountId: UUID) throws -> Record? {
    var query = query(accountId)
    query[kSecReturnData as String] = true
    var result: CFTypeRef?
    let status = backend.copy(query as CFDictionary, result: &result)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess, let data = result as? Data else {
      throw KeychainError(status: status)
    }
    return try JSONDecoder().decode(Record.self, from: data)
  }
  public func save(_ credential: Record, accountId: UUID) throws {
    let data = try JSONEncoder().encode(credential)
    let attributes: [String: Any] = [
      kSecValueData as String: data,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    ]
    let status = backend.update(
      query(accountId) as CFDictionary, attributes: attributes as CFDictionary)
    if status == errSecItemNotFound {
      let status = backend.add(
        query(accountId).merging(attributes, uniquingKeysWith: { _, value in value })
          as CFDictionary)
      guard status == errSecSuccess else { throw KeychainError(status: status) }
    } else if status != errSecSuccess {
      throw KeychainError(status: status)
    }
  }
  public func remove(accountId: UUID) throws {
    let status = backend.delete(query(accountId) as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw KeychainError(status: status)
    }
  }
  public func all() throws -> [Record] {
    var query = query()
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitAll
    var result: CFTypeRef?
    let status = backend.copy(query as CFDictionary, result: &result)
    if status == errSecItemNotFound { return [] }
    guard status == errSecSuccess, let records = result as? [Data] else {
      throw KeychainError(status: status)
    }
    return try records.map { try JSONDecoder().decode(Record.self, from: $0) }
  }
}
