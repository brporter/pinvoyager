import Foundation
import Security
import Testing

@testable import PinVoyagerData

final class StubSecurity: KeychainBackend, @unchecked Sendable {
  let copyStatus: OSStatus
  let copyValue: CFTypeRef?
  let updateStatus: OSStatus
  let addStatus: OSStatus
  let deleteStatus: OSStatus
  init(
    copyStatus: OSStatus = errSecItemNotFound, copyValue: CFTypeRef? = nil,
    updateStatus: OSStatus = errSecItemNotFound, addStatus: OSStatus = errSecSuccess,
    deleteStatus: OSStatus = errSecSuccess
  ) {
    self.copyStatus = copyStatus
    self.copyValue = copyValue
    self.updateStatus = updateStatus
    self.addStatus = addStatus
    self.deleteStatus = deleteStatus
  }
  func copy(_ query: CFDictionary, result: UnsafeMutablePointer<CFTypeRef?>) -> OSStatus {
    result.pointee = copyValue
    return copyStatus
  }
  func update(_ query: CFDictionary, attributes: CFDictionary) -> OSStatus { updateStatus }
  func add(_ attributes: CFDictionary) -> OSStatus { addStatus }
  func delete(_ query: CFDictionary) -> OSStatus { deleteStatus }
}

@Test func credentialFailuresPropagateInsteadOfDroppingOrInventingSessions() async throws {
  let id = UUID()
  let credential = SessionCredential(accountId: id, token: "opaque")
  for status in [errSecInteractionNotAllowed, errSecAuthFailed, errSecMissingEntitlement] {
    let store = KeychainCredentialStore(
      backend: StubSecurity(copyStatus: status, updateStatus: status, deleteStatus: status))
    await #expect(throws: KeychainError.self) { try await store.read(accountId: id) }
    await #expect(throws: KeychainError.self) { try await store.save(credential) }
    await #expect(throws: KeychainError.self) { try await store.all() }
    await #expect(throws: KeychainError.self) { try await store.remove(accountId: id) }
  }
  let unableToAdd = KeychainCredentialStore(backend: StubSecurity(addStatus: errSecNotAvailable))
  await #expect(throws: KeychainError.self) { try await unableToAdd.save(credential) }
  let missing = KeychainCredentialStore(backend: StubSecurity(deleteStatus: errSecItemNotFound))
  #expect(try await missing.read(accountId: id) == nil)
  #expect(try await missing.all().isEmpty)
  try await missing.remove(accountId: id)
  let encoded = try JSONEncoder().encode(credential)
  let readable = KeychainCredentialStore(
    backend: StubSecurity(
      copyStatus: errSecSuccess, copyValue: encoded as CFData, updateStatus: errSecSuccess))
  #expect(try await readable.read(accountId: id) == credential)
  try await readable.save(credential)
  let list = KeychainCredentialStore(
    backend: StubSecurity(copyStatus: errSecSuccess, copyValue: [encoded] as CFArray))
  #expect(try await list.all() == [credential])
  let add = KeychainCredentialStore(backend: StubSecurity())
  try await add.save(credential)
}
