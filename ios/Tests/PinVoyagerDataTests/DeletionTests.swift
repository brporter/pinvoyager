import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

actor MemoryDeletionRecovery: DeletionRecoveryStore {
  var records: [UUID: PendingAccountDeletion] = [:]
  func read(accountId: UUID) -> PendingAccountDeletion? { records[accountId] }
  func save(_ record: PendingAccountDeletion) { records[record.accountId] = record }
  func all() -> [PendingAccountDeletion] { Array(records.values) }
  func remove(accountId: UUID) { records.removeValue(forKey: accountId) }
}

@Test func deletionPreparationSavesRecoveryBeforeSendingAndLostConfirmationRemainsRecoverable()
  async throws
{
  let account = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "session"))
  let recovery = MemoryDeletionRecovery()
  let transport = StubHTTP([])
  let service = AccountDeletionService(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    recovery: recovery)
  await #expect(throws: URLError.self) {
    try await service.prepare(accountId: account, proofToken: "purpose-bound-proof")
  }
  let pending = try #require(await recovery.read(accountId: account))
  #expect(pending.status == .preparing)
  #expect(pending.statusCapability.count >= 43)
  let request = try #require(await transport.requests.first)
  #expect(request.value(forHTTPHeaderField: "Idempotency-Key") == pending.id.uuidString)
  let body = try #require(
    JSONSerialization.jsonObject(with: request.httpBody!) as? [String: String])
  #expect(body["status_capability"] == pending.statusCapability)
  let outcome = StubHTTP([response(200, "{\"status\":\"complete\"}")])
  let reopened = AccountDeletionService(
    client: APIClient(origin: try testOrigin(), transport: outcome, credentials: TestCredentials()),
    recovery: recovery)
  #expect(try await reopened.refresh(accountId: account).status == .complete)
  #expect(
    await outcome.requests.first?.value(forHTTPHeaderField: "Authorization")
      == "Deletion \(pending.statusCapability)")
}

@Test func deletionReauthenticationProducesProofWithoutReplacingAppSession() async throws {
  let account = UUID()
  let attempt = UUID()
  let credentials = TestCredentials()
  await credentials.save(SessionCredential(accountId: account, token: "existing-session"))
  let transport = StubHTTP([
    response(
      201,
      "{\"attempt_id\":\"\(attempt)\",\"browser_start_url\":\"https://pinvoyager.com/auth/native/attempts/\(attempt)/start\"}"
    ),
    response(
      200,
      "{\"proof_token\":\"proof\",\"account_id\":\"\(account)\",\"provider\":\"apple\",\"purpose\":\"account_deletion\",\"expires_at\":\"2026-10-26T12:00:00Z\"}"
    ),
  ])
  let authentication = NativeAuthentication(
    client: APIClient(origin: try testOrigin(), transport: transport, credentials: credentials),
    credentials: credentials)
  let pending = try await authentication.beginReauthentication(
    provider: "apple", accountId: account)
  let proof = try await authentication.completeReauthentication(
    URL(string: "pinvoyager://auth/callback?state=\(pending.state)&code=one-time")!,
    pending: pending)
  #expect(proof.accountId == account)
  #expect(proof.proofToken == "proof")
  #expect(await credentials.read(accountId: account)?.token == "existing-session")
  #expect(await transport.requests.first?.url?.path == "/auth/native/reauth-attempts")
}

@Test func serverTimestampsAcceptFractionalAndWholeSeconds() {
  #expect(APIJSON.date("2026-10-25T12:34:56.123456Z") != nil)
  #expect(APIJSON.date("2026-10-25T12:34:56Z") != nil)
  #expect(APIJSON.date("not-a-date") == nil)
}

@Test func preparationCannotOverwriteAnUncertainDeletionConfirmation() async throws {
  let account = UUID()
  let recovery = MemoryDeletionRecovery()
  let record = PendingAccountDeletion(
    id: UUID(), accountId: account, statusCapability: "saved-private-capability", proofToken: nil,
    appleProofToken: nil, status: .confirming, expiresAt: Date().addingTimeInterval(300))
  await recovery.save(record)
  let transport = StubHTTP([])
  let service = AccountDeletionService(
    client: APIClient(
      origin: try testOrigin(), transport: transport, credentials: TestCredentials()),
    recovery: recovery)
  await #expect(throws: ValidationError.self) {
    try await service.prepare(accountId: account, proofToken: "new-proof")
  }
  #expect(await recovery.read(accountId: account) == record)
  #expect(await transport.requests.isEmpty)
}

@Test func uncertainPreparationKeepsItsRecoveryCapabilityUntilItCanBeRetried() async throws {
  let account = UUID()
  let recovery = MemoryDeletionRecovery()
  let service = AccountDeletionService(
    client: APIClient(
      origin: try testOrigin(), transport: StubHTTP([]), credentials: TestCredentials()),
    recovery: recovery)
  _ = try? await service.prepare(accountId: account, proofToken: "first-proof")
  let original = try #require(await recovery.read(accountId: account))
  await #expect(throws: ValidationError.self) {
    try await service.prepare(accountId: account, proofToken: "replacement-proof")
  }
  #expect(await recovery.read(accountId: account) == original)
}
