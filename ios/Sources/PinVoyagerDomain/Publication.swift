import Foundation

public enum PhotoReference: Codable, Equatable, Sendable {
  case photo(id: UUID)
  case upload(key: UUID, contentHash: String)
}

public enum PublicationStatus: String, Codable, Sendable {
  case queued
  case waitingForUploads = "waiting_for_uploads"
  case processing, published, canceled, expired
  case needsReview = "needs_review"
  case failed
}

public struct PublicationIntent: Codable, Equatable, Identifiable, Sendable {
  public let id: UUID
  public let voyageId: UUID
  public var serverId: UUID?
  public let fields: VoyageFields
  public let expectedRevision: String?
  public let photoReferences: [PhotoReference]
  public let consentAt: Date
  public let expiresAt: Date
  public var status: PublicationStatus = .queued
  public var cancellationPending = false
  public var notifiedAt: Date?

  public var isArmed: Bool {
    serverId != nil && [.waitingForUploads, .processing].contains(status)
  }

  public static func review(
    voyage: LocalVoyage, photos: [LocalPhoto], at date: Date, maximumPhotos: Int = 10_000
  ) throws -> Self {
    let fields = try voyage.fields.validated(isNew: true)
    guard !voyage.confirmedPublished, !voyage.deleted, fields.granularity.canPublish else {
      throw ValidationError(
        field: "publication",
        message: "Review a private voyage with a supported location precision.")
    }
    let included = photos.filter { !$0.deleted }
    guard !included.isEmpty, included.count <= maximumPhotos,
      included.allSatisfy({
        $0.voyageId == voyage.id && $0.isDurable && $0.variant != .failed
          && $0.variant != .unsupported
      })
    else {
      throw ValidationError(
        field: "photos",
        message: "Save at least one photo and resolve missing or failed files before publication.")
    }
    let references: [PhotoReference] = included.map {
      if let id = $0.serverId { return .photo(id: id) }
      return .upload(key: $0.uploadKey, contentHash: $0.source!.contentHash)
    }
    return Self(
      id: UUID(), voyageId: voyage.id, fields: fields, expectedRevision: voyage.revision,
      photoReferences: references,
      consentAt: date, expiresAt: date.addingTimeInterval(7 * 24 * 60 * 60))
  }
}
