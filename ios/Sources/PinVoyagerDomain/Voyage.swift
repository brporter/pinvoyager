import Foundation

public struct ValidationError: Error, Equatable, LocalizedError, Sendable {
  public let field: String
  public let message: String
  public var errorDescription: String? { message }
  public init(field: String, message: String) {
    self.field = field
    self.message = message
  }
}

public struct VoyageFields: Codable, Equatable, Sendable {
  public var title: String
  public var description: String?
  public var granularity: Granularity

  public init(title: String = "", description: String? = "", granularity: Granularity = .km1) {
    self.title = title
    self.description = description
    self.granularity = granularity
  }

  public func validated(isNew: Bool) throws -> Self {
    var fields = self
    fields.title = title.trimmingCharacters(in: .whitespacesAndNewlines)
    fields.description = description?.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !fields.title.isEmpty else {
      throw ValidationError(field: "title", message: "Enter a voyage name.")
    }
    guard fields.title.unicodeScalars.count <= 200 else {
      throw ValidationError(
        field: "title", message: "Use at most 200 characters for the voyage name.")
    }
    guard (fields.description?.unicodeScalars.count ?? 0) <= 8_000 else {
      throw ValidationError(
        field: "description", message: "Use at most 8,000 characters for the description.")
    }
    if isNew && (fields.description ?? "").isEmpty {
      throw ValidationError(field: "description", message: "Enter a description.")
    }
    return fields
  }
}

public enum DisplayName {
  public static func normalize(_ value: String) throws -> String {
    let name = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    guard name.range(of: "^[a-z][a-z0-9-]{1,28}[a-z0-9]$", options: .regularExpression) != nil
    else {
      throw ValidationError(
        field: "display_name",
        message:
          "Use 3–30 letters, digits or hyphens. Start with a letter and end with a letter or digit."
      )
    }
    return name
  }
}

public struct VoyageDeletionUndo: Codable, Equatable, Sendable {
  public let photoIds: Set<UUID>
  public let operationStates: [UUID: OperationState]
  public init(photoIds: Set<UUID>, operationStates: [UUID: OperationState]) {
    self.photoIds = photoIds
    self.operationStates = operationStates
  }
}

public struct LocalVoyage: Codable, Equatable, Identifiable, Sendable {
  public let id: UUID
  public let ownerId: UUID
  public var serverId: UUID?
  public var fields: VoyageFields
  public var serverFields: VoyageFields?
  public var revision: String?
  public var localSequence: Int = 0
  public var confirmedPublished = false
  public var deleted = false
  public var deletionUndo: VoyageDeletionUndo?
  public var uploadsPaused: Bool?
  public var updatedAt: Date

  public init(id: UUID = UUID(), ownerId: UUID, fields: VoyageFields, at: Date = Date()) {
    self.id = id
    self.ownerId = ownerId
    self.fields = fields
    self.updatedAt = at
  }
}

public struct VoyageWorkingCopy: Codable, Equatable, Identifiable, Sendable {
  public var id: UUID { voyageId }
  public let voyageId: UUID
  public let base: VoyageFields
  public let baseRevision: String?
  public var fields: VoyageFields
  public init(voyage: LocalVoyage, fields: VoyageFields) {
    voyageId = voyage.id
    base = voyage.fields
    baseRevision = voyage.revision
    self.fields = fields
  }
}

public enum FieldResolution: String, Codable, Sendable { case local, remote, needsChoice }

public struct FieldComparison<Value: Equatable & Sendable>: Sendable {
  public let base: Value
  public let local: Value
  public let remote: Value
  public var resolution: FieldResolution {
    local == remote || remote == base ? .local : local == base ? .remote : .needsChoice
  }
  public init(base: Value, local: Value, remote: Value) {
    self.base = base
    self.local = local
    self.remote = remote
  }
}
