import Foundation

public enum NetworkState: Equatable, Sendable {
  case offline
  case wifi(constrained: Bool)
  case cellular(constrained: Bool)
}
public enum WaitReason: String, Codable, Sendable {
  case offline, wifi, constrained, signIn, paused, dependency, retryDelay, system
}

public enum SyncPreference: CaseIterable, Sendable {
  case wifiOnly, allowsConstrained, backgroundPreparation, paused, keepOriginals, uploadsPaused
  public var keyPath: WritableKeyPath<SyncPolicy, Bool> {
    switch self {
    case .wifiOnly: \.wifiOnly
    case .allowsConstrained: \.allowsConstrained
    case .backgroundPreparation: \.backgroundPreparation
    case .paused: \.paused
    case .uploadsPaused: \.areUploadsPaused
    case .keepOriginals: \.keepOriginals
    }
  }
}

public struct SyncPolicy: Codable, Equatable, Sendable {
  public var wifiOnly = true
  public var allowsConstrained = false
  public var backgroundPreparation = true
  public var keepOriginals = true
  public var notifyOnPublication: Bool?
  public var paused = false
  public var uploadsPaused: Bool?
  public var areUploadsPaused: Bool {
    get { uploadsPaused == true }
    set { uploadsPaused = newValue }
  }
  public init() {}
  public func forUpload(cellularException: Bool) -> Self {
    var policy = self
    if cellularException { policy.wifiOnly = false }
    policy.paused = paused || areUploadsPaused
    return policy
  }
  public func waitReason(network: NetworkState, authenticated: Bool) -> WaitReason? {
    if paused { return .paused }
    if !authenticated { return .signIn }
    switch network {
    case .offline: return .offline
    case .cellular where wifiOnly: return .wifi
    case .wifi(let constrained), .cellular(let constrained):
      return constrained && !allowsConstrained ? .constrained : nil
    }
  }
}

public enum ResponseDisposition: Equatable, Sendable {
  case success, signIn, conflict, reconcile, destinationMissing, attention
  public static func classify(status: Int) -> Self {
    switch status {
    case 200..<300: .success
    case 401: .signIn
    case 404: .destinationMissing
    case 412: .conflict
    case 408, 429, 500...599: .reconcile
    default: .attention
    }
  }
}

public enum RetrySchedule {
  public static func delay(attempt: Int, randomUnit: Double, retryAfter: TimeInterval = 0)
    -> TimeInterval
  {
    max(retryAfter, min(900, pow(2, Double(min(30, max(1, attempt))))) * min(1, max(0, randomUnit)))
  }
}

public enum OperationState: String, Codable, Sendable {
  case waiting, submitted, uncertain, applied, attention, conflict, canceled
}

public enum OperationKind: Codable, Equatable, Sendable {
  case create(VoyageFields)
  case edit(VoyageFields)
  case upload(photoId: UUID, publicConsent: Bool)
  case order([UUID])
  case removePhoto(photoId: UUID, uploadKey: UUID)
  case deleteVoyage, unpublish
  case publish(intentId: UUID)
  case cancelPublication(intentId: UUID)
  case block(userId: UUID, blocked: Bool)
  case report(target: String, reason: String, details: String)
  public var priority: Int {
    switch self {
    case .cancelPublication, .unpublish, .deleteVoyage, .removePhoto: 0
    case .create, .edit, .block, .report: 1
    case .publish: 2
    case .upload: 3
    case .order: 4
    }
  }
}

public struct MutationRequest: Codable, Equatable, Sendable {
  public let method: String
  public let path: String
  public let body: Data?
  public let revision: String?
  public var headers: [String: String]
  public init(
    method: String, path: String, body: Data? = nil, revision: String? = nil,
    headers: [String: String] = [:]
  ) {
    self.method = method
    self.path = path
    self.body = body
    self.revision = revision
    self.headers = headers
  }
}

public struct OutboxOperation: Codable, Equatable, Identifiable, Sendable {
  public enum Scope: Hashable, Sendable {
    case voyage(UUID)
    case block(UUID)
    case operation(UUID)
  }
  public var scope: Scope {
    if let voyageId { return .voyage(voyageId) }
    if case .block(let userId, _) = kind { return .block(userId) }
    return .operation(id)
  }
  public let id: UUID
  public let accountId: UUID
  public let generation: Int
  public let voyageId: UUID?
  public var kind: OperationKind
  public var expectedRevision: String?
  public var dependencies: Set<UUID>
  public var state: OperationState = .waiting
  public var request: MutationRequest?
  public var attempts = 0
  public var nextAttempt = Date.distantPast
  public var lastError: String?
  public var receiptPreviousRevision: String?
  public var receiptRevision: String?
  public var receiptRejected: Bool?
  public var completedAt: Date?
  public var requestId: String?
  public init(
    id: UUID = UUID(), accountId: UUID, generation: Int, voyageId: UUID?, kind: OperationKind,
    expectedRevision: String? = nil, dependencies: Set<UUID> = []
  ) {
    self.id = id
    self.accountId = accountId
    self.generation = generation
    self.voyageId = voyageId
    self.kind = kind
    self.expectedRevision = expectedRevision
    self.dependencies = dependencies
  }
}

public struct BackgroundTransfer: Codable, Equatable, Identifiable, Sendable {
  public var id: UUID { operationId }
  public let operationId: UUID
  public let generation: Int
  public let sessionIdentifier: String
  public var taskId: Int?
  public let bodyPath: String
  public let byteCount: Int64
  public var completed = false
  public init(
    operationId: UUID, generation: Int, sessionIdentifier: String, bodyPath: String,
    byteCount: Int64
  ) {
    self.operationId = operationId
    self.generation = generation
    self.sessionIdentifier = sessionIdentifier
    self.bodyPath = bodyPath
    self.byteCount = byteCount
  }
}
