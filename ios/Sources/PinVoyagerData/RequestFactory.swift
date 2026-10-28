import CryptoKit
import Foundation
import PinVoyagerDomain

public enum RequestFactory {
  public static func metadataDigest(_ fields: VoyageFields) throws -> String {
    let data = try JSONSerialization.data(
      withJSONObject: [
        fields.title, fields.description as Any? ?? NSNull(), fields.granularity.rawValue,
      ], options: [.withoutEscapingSlashes])
    return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
  }

  public static func prepare(_ operation: OutboxOperation, workspace: Workspace, now: Date) throws
    -> MutationRequest
  {
    let voyage = workspace.voyages.first { $0.id == operation.voyageId }
    let path = voyage?.serverId.map { "/api/voyages/\($0)" }
    func body(_ object: [String: Any]) throws -> Data {
      try JSONSerialization.data(
        withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes])
    }
    func fields(_ fields: VoyageFields) throws -> Data {
      try body([
        "title": fields.title, "description": fields.description as Any? ?? NSNull(),
        "granularity": fields.granularity.rawValue,
      ])
    }
    func remotePath() throws -> String {
      guard let path else { throw WorkspaceError.missingVoyage }
      return path
    }
    func revision() throws -> String {
      guard let value = operation.expectedRevision ?? voyage?.revision,
        value.range(of: "^[0-9]+$", options: .regularExpression) != nil
      else { throw WorkspaceError.missingVoyage }
      return value
    }
    switch operation.kind {
    case .create(let value):
      return MutationRequest(method: "POST", path: "/api/voyages", body: try fields(value))
    case .edit(let value):
      return MutationRequest(
        method: "PATCH", path: try remotePath(), body: try fields(value), revision: try revision())
    case .unpublish:
      return MutationRequest(
        method: "PATCH", path: try remotePath(), body: try body(["published": false]),
        revision: try revision())
    case .deleteVoyage:
      return MutationRequest(method: "DELETE", path: try remotePath(), revision: try revision())
    case .upload(let photoId, let publicConsent):
      guard let photo = workspace.photos.first(where: { $0.id == photoId }), !photo.deleted,
        photo.source != nil
      else { throw FileVaultError.missingSource }
      return MutationRequest(
        method: "POST", path: try remotePath() + "/photos",
        headers: [
          "Idempotency-Key": photo.uploadKey.uuidString,
          "X-PinVoyager-Upload-Visibility": publicConsent ? "public" : "draft",
        ])
    case .removePhoto(let photoId, let key):
      if let serverId = workspace.photos.first(where: { $0.id == photoId })?.serverId {
        return MutationRequest(
          method: "DELETE", path: try remotePath() + "/photos/\(serverId)", revision: try revision()
        )
      }
      return MutationRequest(method: "DELETE", path: try remotePath() + "/uploads/\(key)")
    case .order(let ids):
      let mapped = try ids.map { id -> String in
        guard let serverId = workspace.photos.first(where: { $0.id == id && !$0.deleted })?.serverId
        else { throw WorkspaceError.invalidOrder }
        return serverId.uuidString
      }
      return MutationRequest(
        method: "PUT", path: try remotePath() + "/photo-order",
        body: try body(["photo_ids": mapped]), revision: try revision())
    case .publish(let intentId):
      guard let intent = workspace.publicationIntents.first(where: { $0.id == intentId }),
        intent.status == .queued,
        intent.expiresAt > now, intent.fields == voyage?.fields
      else {
        throw ValidationError(
          field: "publication",
          message: "Review publication again. Its saved consent changed or expired.")
      }
      let references = intent.photoReferences.map { reference -> [String: String] in
        switch reference {
        case .photo(let id): ["photo_id": id.uuidString]
        case .upload(let key, let hash): ["upload_key": key.uuidString, "content_hash": hash]
        }
      }
      return MutationRequest(
        method: "POST", path: try remotePath() + "/publication-intents",
        body: try body([
          "metadata_digest": metadataDigest(intent.fields),
          "expires_at": ISO8601DateFormatter().string(from: intent.expiresAt), "photos": references,
        ]), revision: try revision())
    case .cancelPublication(let intentId):
      guard let id = workspace.publicationIntents.first(where: { $0.id == intentId })?.serverId
      else { throw WorkspaceError.missingVoyage }
      return MutationRequest(
        method: "DELETE", path: try remotePath() + "/publication-intents/\(id)")
    case .block(let userId, let blocked):
      return MutationRequest(method: blocked ? "PUT" : "DELETE", path: "/api/me/blocks/\(userId)")
    case .report(let target, let reason, let details):
      return MutationRequest(
        method: "POST", path: "/api/reports",
        body: try body(["target": target, "reason": reason, "details": details]))
    }
  }
}

public struct NativeSyncRemote: SyncRemote {
  public let client: APIClient
  public let accountId: UUID
  public init(client: APIClient, accountId: UUID) {
    self.client = client
    self.accountId = accountId
  }
  public func supportsNativeSync() async throws -> Bool {
    try await client.capabilities()?.supportsNativeSync == true
  }

  private struct ReceiptDTO: Decodable, Sendable {
    let status: String
    let targetId: UUID?
    let photoId: UUID?
    let revision: String?
    let previousRevision: String?
    let message: String?
    let responseStatus: Int?
    let voyageId: UUID?
    let publicationIntentId: UUID?
    let publicationStatus: PublicationStatus?
    func value() throws -> OperationReceipt {
      switch status {
      case "not_seen": return .notSeen
      case "in_progress": return .inProgress
      case "applied", "accepted":
        if let id = publicationIntentId, let status = publicationStatus, let revision {
          return .publication(
            intentId: id, status: status, revision: revision, previousRevision: previousRevision)
        }
        guard let id = targetId ?? photoId, let revision else {
          throw WorkspaceError.malformedReceipt
        }
        return .applied(targetId: id, revision: revision, previousRevision: previousRevision)
      case "deleted", "canceled":
        return revision.map { .removed(revision: $0, previousRevision: previousRevision) }
          ?? .deleted
      case "rejected":
        return .rejected(message ?? "This change needs attention.", status: responseStatus)
      default: throw WorkspaceError.unsupportedProtocol
      }
    }
  }

  public func receipt(operationId: UUID) async throws -> OperationReceipt {
    let dto: ReceiptDTO = try await client.get(
      path: "/api/sync/operations/\(operationId)", accountId: accountId)
    if dto.publicationStatus == .published {
      guard let voyage = dto.voyageId, let intent = dto.publicationIntentId,
        let revision = dto.revision
      else {
        throw WorkspaceError.malformedReceipt
      }
      struct Current: Decodable, Sendable {
        let published: Bool
      }
      let current: Current = try await client.get(
        path: "/api/voyages/\(voyage)", accountId: accountId)
      return .publication(
        intentId: intent, status: .published, revision: revision,
        confirmedPublished: current.published, previousRevision: dto.previousRevision)
    }
    return try dto.value()
  }
  public func uploadReceipt(voyageId: UUID, key: UUID) async throws -> OperationReceipt {
    let dto: ReceiptDTO = try await client.get(
      path: "/api/voyages/\(voyageId)/uploads/\(key)", accountId: accountId)
    return try dto.value()
  }
  public func isPublished(voyageId: UUID) async throws -> Bool {
    struct Visibility: Decodable, Sendable { let published: Bool }
    let current: Visibility = try await client.get(
      path: "/api/voyages/\(voyageId)", accountId: accountId)
    return current.published
  }
  public func publicationStatus(voyageId: UUID, intentId: UUID) async throws -> OperationReceipt {
    struct Status: Decodable, Sendable {
      let id: UUID
      let status: PublicationStatus
      let revision: String
      let published: Bool
    }
    let result: Status = try await client.get(
      path: "/api/voyages/\(voyageId)/publication-intents/\(intentId)", accountId: accountId)
    return .publication(
      intentId: result.id, status: result.status, revision: result.revision,
      confirmedPublished: result.published)
  }
  public func submit(_ request: MutationRequest, operationId: UUID) async throws -> OperationReceipt
  {
    var headers = request.headers
    if headers["Idempotency-Key"] == nil { headers["Idempotency-Key"] = operationId.uuidString }
    if let revision = request.revision { headers["If-Match"] = "\"v:\(revision)\"" }
    let response = try await client.raw(
      method: request.method, path: request.path, body: request.body, accountId: accountId,
      headers: headers)
    // Mutations may return resource bodies or no body. Only the durable receipt establishes their outcome.
    guard (200..<300).contains(response.status) else { throw WorkspaceError.malformedReceipt }
    return try await receipt(operationId: operationId)
  }
}
