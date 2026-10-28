import Foundation

public struct SourceFile: Codable, Equatable, Sendable {
  public let relativePath: String
  public let contentHash: String
  public let byteCount: Int64
  public let mediaType: String
  public init(relativePath: String, contentHash: String, byteCount: Int64, mediaType: String) {
    self.relativePath = relativePath
    self.contentHash = contentHash
    self.byteCount = byteCount
    self.mediaType = mediaType
  }
}

public enum TransferState: String, Codable, Sendable {
  case acquiring, needsSource, saved, waiting, scheduled, uploading, checkingReceipt, accepted,
    needsAttention, removing, removed
}

public enum VariantState: String, Codable, Sendable {
  case pending, ready, failed, unsupported
  public init(from decoder: any Decoder) throws {
    self = Self(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unsupported
  }
}

public struct OfflinePhotoCopy: Codable, Equatable, Sendable {
  public let revision: String
  public let variants: [String: String]
  public let byteCount: Int64
  public let savedAt: Date
  public init(
    revision: String, variants: [String: String], byteCount: Int64, savedAt: Date = Date()
  ) {
    self.revision = revision
    self.variants = variants
    self.byteCount = byteCount
    self.savedAt = savedAt
  }
}

public struct LocalPhoto: Codable, Equatable, Identifiable, Sendable {
  public let id: UUID
  public let voyageId: UUID
  public let uploadKey: UUID
  public var serverId: UUID?
  public var filename: String
  public var replacesPhotoId: UUID?
  public var importPublicConsent: Bool?
  public var importBatchId: UUID?
  public var cellularException: Bool?
  public var source: SourceFile?
  public var previewPath: String?
  public var offlineCopy: OfflinePhotoCopy?
  public var coordinate: Coordinate?
  public var capturedAt: String?
  public var width = 0
  public var height = 0
  public var order = 0
  public var transfer: TransferState = .acquiring
  public var variant: VariantState = .pending
  public var deleted = false
  public var error: String?
  public var canRetryUpload: Bool { transfer == .needsAttention && variant != .failed && !deleted }
  public var isDurable: Bool { source != nil || serverId != nil }
  public init(id: UUID = UUID(), voyageId: UUID, uploadKey: UUID = UUID(), filename: String) {
    self.id = id
    self.voyageId = voyageId
    self.uploadKey = uploadKey
    self.filename = filename
  }
}
