import CryptoKit
import Foundation
import PinVoyagerDomain
import Security

public enum AuthenticationError: Error, LocalizedError, Sendable {
  case invalidCallback, invalidAttempt, signInRequired, canceled
  public var errorDescription: String? {
    switch self {
    case .invalidCallback, .invalidAttempt:
      "Sign-in could not be verified. Start a new sign-in attempt."
    case .signInRequired: "Sign in to resume syncing."
    case .canceled: "Sign-in canceled."
    }
  }
}

public struct PendingAuthentication: Sendable {
  public let id: UUID
  public let state: String
  public let verifier: String
  public let browserStartUrl: URL
  public let linkingAccountId: UUID?
  public let deletionAccountId: UUID?
}

public struct SessionExchange: Decodable, Sendable {
  public let sessionToken: String
  public let user: UserProfile
  public let identities: [LinkedIdentity]
  public let idleDeadline: String
  public let absoluteDeadline: String
}

public actor NativeAuthentication {
  private let client: APIClient
  private let credentials: any CredentialStore
  private var pendingIds: Set<UUID> = []
  public init(client: APIClient, credentials: any CredentialStore) {
    self.client = client
    self.credentials = credentials
  }

  public func begin(provider: String, linkingAccountId: UUID? = nil) async throws
    -> PendingAuthentication
  {
    try await begin(provider: provider, linkingAccountId: linkingAccountId, deletionAccountId: nil)
  }

  public func beginReauthentication(provider: String, accountId: UUID) async throws
    -> PendingAuthentication
  {
    try await begin(provider: provider, linkingAccountId: nil, deletionAccountId: accountId)
  }

  private func begin(provider: String, linkingAccountId: UUID?, deletionAccountId: UUID?)
    async throws -> PendingAuthentication
  {
    guard ["apple", "google", "microsoft"].contains(provider) else {
      throw AuthenticationError.invalidAttempt
    }
    let state = try Self.randomToken()
    let verifier = try Self.randomToken()
    struct Body: Encodable {
      let provider: String
      let mode: String?
      let purpose: String?
      let state: String
      let challenge: String
      let callbackIdentifier = "pinvoyager"
    }
    struct Reply: Decodable, Sendable {
      let attemptId: UUID
      let browserStartUrl: URL
    }
    let challenge = Self.base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
    let reply: Reply = try await client.send(
      method: "POST",
      path: deletionAccountId == nil ? "/auth/native/attempts" : "/auth/native/reauth-attempts",
      body: APIJSON.encode(
        Body(
          provider: provider,
          mode: deletionAccountId == nil ? (linkingAccountId == nil ? "sign_in" : "link") : nil,
          purpose: deletionAccountId == nil ? nil : "account_deletion", state: state,
          challenge: challenge)), accountId: linkingAccountId ?? deletionAccountId)
    let path = reply.browserStartUrl.path.split(separator: "/")
    guard client.origin.contains(reply.browserStartUrl), path.count == 5,
      path[0] == "auth", path[1] == "native", path[2] == "attempts", path[4] == "start",
      UUID(uuidString: String(path[3])) == reply.attemptId, reply.browserStartUrl.query == nil,
      reply.browserStartUrl.fragment == nil
    else { throw AuthenticationError.invalidAttempt }
    pendingIds.insert(reply.attemptId)
    return PendingAuthentication(
      id: reply.attemptId, state: state, verifier: verifier, browserStartUrl: reply.browserStartUrl,
      linkingAccountId: linkingAccountId, deletionAccountId: deletionAccountId)
  }

  public func complete(_ callback: URL, pending: PendingAuthentication) async throws
    -> SessionExchange
  {
    guard pending.deletionAccountId == nil else { throw AuthenticationError.invalidAttempt }
    let body = try exchangeBody(callback, pending: pending)
    let reply: SessionExchange = try await client.send(
      method: "POST", path: "/auth/native/exchange",
      body: APIJSON.encode(body))
    guard !reply.sessionToken.isEmpty,
      pending.linkingAccountId == nil || pending.linkingAccountId == reply.user.id
    else { throw WorkspaceError.wrongAccount }
    var credential = SessionCredential(accountId: reply.user.id, token: reply.sessionToken)
    credential.idleDeadline = reply.idleDeadline
    credential.absoluteDeadline = reply.absoluteDeadline
    try await credentials.save(credential)
    return reply
  }

  public func completeReauthentication(_ callback: URL, pending: PendingAuthentication) async throws
    -> DeletionProof
  {
    guard let accountId = pending.deletionAccountId else {
      throw AuthenticationError.invalidAttempt
    }
    let body = try exchangeBody(callback, pending: pending)
    let proof: DeletionProof = try await client.send(
      method: "POST", path: "/auth/native/exchange", body: APIJSON.encode(body))
    guard proof.accountId == accountId, proof.purpose == "account_deletion",
      !proof.proofToken.isEmpty
    else { throw WorkspaceError.wrongAccount }
    return proof
  }
  private struct ExchangeBody: Encodable {
    let attemptId: UUID
    let code: String
    let verifier: String
  }
  private func exchangeBody(_ callback: URL, pending: PendingAuthentication) throws -> ExchangeBody
  {
    guard pendingIds.contains(pending.id), callback.scheme == "pinvoyager", callback.host == "auth",
      callback.path == "/callback",
      callback.user == nil, callback.password == nil, callback.port == nil,
      callback.fragment == nil,
      let items = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems,
      items.count == 2, Set(items.map(\.name)) == ["state", "code"],
      items.first(where: { $0.name == "state" })?.value == pending.state,
      let code = items.first(where: { $0.name == "code" })?.value, !code.isEmpty
    else { throw AuthenticationError.invalidCallback }
    pendingIds.remove(pending.id)
    return ExchangeBody(attemptId: pending.id, code: code, verifier: pending.verifier)
  }

  public func cancel(_ pending: PendingAuthentication) { pendingIds.remove(pending.id) }

  public func revokePendingCredentials() async throws {
    for credential in try await credentials.all() where credential.revocationOnly {
      _ = try await client.raw(method: "POST", path: "/auth/native/logout", token: credential.token)
      try await credentials.remove(accountId: credential.accountId)
    }
  }

  static func base64URL(_ bytes: Data) -> String {
    bytes.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(
      of: "/", with: "_"
    ).replacingOccurrences(of: "=", with: "")
  }
  static func randomToken() throws -> String {
    var bytes = [UInt8](repeating: 0, count: 32)
    let status = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
    guard status == errSecSuccess else { throw KeychainError(status: status) }
    return base64URL(Data(bytes))
  }
}
