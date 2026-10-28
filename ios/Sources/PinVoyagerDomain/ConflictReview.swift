import Foundation

public enum PhotoConflictItem: Codable, Hashable, Sendable {
  case website(UUID)
  case local(UUID)
}

public enum PhotoConflictChoice: Codable, Equatable, Sendable {
  case website, websiteAppendingLocal
  case arranged([PhotoConflictItem])
}

public struct ConflictReview: Codable, Equatable, Identifiable, Sendable {
  public var id: UUID { voyageId }
  public let voyageId: UUID
  public var generation: Int
  public var mine: VoyageFields
  public var website: VoyageFields
  public var revision: String
  public var published: Bool
  public var websitePhotos: [UUID]
  public var choices: [String: FieldResolution] = [:]
  public var photoChoice: PhotoConflictChoice?
  public var readded: Set<UUID> = []
  public var makePrivate: Bool?
  public init(
    voyageId: UUID, generation: Int, mine: VoyageFields, website: VoyageFields,
    revision: String, published: Bool, websitePhotos: [UUID]
  ) {
    self.voyageId = voyageId
    self.generation = generation
    self.mine = mine
    self.website = website
    self.revision = revision
    self.published = published
    self.websitePhotos = websitePhotos
  }
  public var resolvedFields: VoyageFields {
    VoyageFields(
      title: choices["title"] == .local ? mine.title : website.title,
      description: choices["description"] == .local ? mine.description : website.description,
      granularity: choices["granularity"] == .local ? mine.granularity : website.granularity)
  }
  public var fieldsChosen: Bool {
    ["title", "description", "granularity"].allSatisfy {
      choices[$0] == .local || choices[$0] == .remote
    }
  }
}
