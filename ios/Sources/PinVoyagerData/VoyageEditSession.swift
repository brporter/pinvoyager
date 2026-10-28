import Foundation
import PinVoyagerDomain

/// An editor keeps its original namespace even if another window switches accounts.
public struct VoyageEditSession: Sendable {
  public let accountId: UUID
  public let voyageId: UUID?
  private let repository: LocalRepository
  public init(accountId: UUID, voyageId: UUID? = nil, repository: LocalRepository) {
    self.accountId = accountId
    self.voyageId = voyageId
    self.repository = repository
  }
  public func saveWorkingCopy(_ fields: VoyageFields) async throws {
    if let voyageId {
      try await repository.saveWorkingCopy(accountId: accountId, voyageId: voyageId, fields: fields)
    } else {
      try await repository.saveScratch(accountId: accountId, fields: fields)
    }
  }
  public func discardWorkingCopy() async throws {
    if let voyageId {
      try await repository.discardWorkingCopy(accountId: accountId, voyageId: voyageId)
    } else {
      try await repository.saveScratch(accountId: accountId, fields: nil)
    }
  }
  public func commit(_ fields: VoyageFields) async throws -> UUID {
    if let voyageId {
      try await repository.editVoyage(accountId: accountId, voyageId: voyageId, fields: fields)
      return voyageId
    }
    return try await repository.createVoyage(accountId: accountId, fields: fields)
  }
}
