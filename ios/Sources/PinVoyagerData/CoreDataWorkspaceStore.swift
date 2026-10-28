import CoreData
import Foundation
import PinVoyagerDomain

/// One private context serializes whole workspace transactions. Only value types leave its queue.
public final class CoreDataWorkspaceStore: WorkspaceStore, @unchecked Sendable {
  private let coordinator: NSPersistentStoreCoordinator
  private let context: NSManagedObjectContext
  private static let collections = [
    "Account", "Voyage", "Photo", "Operation", "WorkingCopy", "Publication", "Transfer",
    "Preferences",
  ]

  public init(url: URL?) throws {
    let model = NSManagedObjectModel()
    model.versionIdentifiers = ["PinVoyager.1"]
    model.entities = Self.collections.map { name in
      let entity = NSEntityDescription()
      entity.name = name
      entity.managedObjectClassName = "NSManagedObject"
      entity.properties = [
        ("id", NSAttributeType.UUIDAttributeType), ("accountId", .UUIDAttributeType),
        ("payload", .binaryDataAttributeType),
      ].map { name, type in
        let attribute = NSAttributeDescription()
        attribute.name = name
        attribute.attributeType = type
        attribute.isOptional = false
        return attribute
      }
      entity.uniquenessConstraints = [["accountId", "id"]]
      return entity
    }
    coordinator = NSPersistentStoreCoordinator(managedObjectModel: model)
    var options: [AnyHashable: Any] = [
      NSMigratePersistentStoresAutomaticallyOption: true,
      NSInferMappingModelAutomaticallyOption: true,
    ]
    if let url {
      var directory = url.deletingLastPathComponent()
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      var values = URLResourceValues()
      values.isExcludedFromBackup = true
      try directory.setResourceValues(values)
      #if os(iOS)
        options[NSPersistentStoreFileProtectionKey] =
          FileProtectionType.completeUntilFirstUserAuthentication
      #endif
    }
    try coordinator.addPersistentStore(
      ofType: url == nil ? NSInMemoryStoreType : NSSQLiteStoreType, configurationName: nil, at: url,
      options: options)
    context = NSManagedObjectContext(concurrencyType: .privateQueueConcurrencyType)
    context.persistentStoreCoordinator = coordinator
    context.mergePolicy = NSMergePolicy(merge: .errorMergePolicyType)
    context.undoManager = nil
  }

  public func accountIds() async throws -> [UUID] {
    try await context.perform {
      try self.rows("Account", accountId: nil).compactMap { $0.value(forKey: "id") as? UUID }
    }
  }

  public func insert(_ workspace: Workspace) async throws {
    try await context.perform {
      guard try self.rows("Account", accountId: workspace.account.id).isEmpty else {
        throw WorkspaceError.wrongAccount
      }
      try self.commit(workspace)
    }
  }

  public func read(accountId: UUID) async throws -> Workspace {
    try await context.perform { try self.load(accountId) }
  }

  public func update(
    accountId: UUID, _ change: @escaping @Sendable (inout Workspace) throws -> Void
  ) async throws {
    try await context.perform {
      do {
        var workspace = try self.load(accountId)
        try change(&workspace)
        guard workspace.account.id == accountId,
          workspace.voyages.allSatisfy({ $0.ownerId == accountId }),
          workspace.operations.allSatisfy({ $0.accountId == accountId })
        else { throw WorkspaceError.wrongAccount }
        try self.commit(workspace)
      } catch {
        self.context.rollback()
        throw error
      }
    }
  }

  public func remove(accountId: UUID) async throws {
    try await context.perform {
      do {
        for name in Self.collections {
          for row in try self.rows(name, accountId: accountId) { self.context.delete(row) }
        }
        try self.context.save()
      } catch {
        self.context.rollback()
        throw error
      }
    }
  }

  private func rows(_ entity: String, accountId: UUID?) throws -> [NSManagedObject] {
    let request = NSFetchRequest<NSManagedObject>(entityName: entity)
    if let accountId {
      request.predicate = NSPredicate(format: "accountId == %@", accountId as NSUUID)
    }
    return try context.fetch(request)
  }

  private func decode<T: Decodable>(_ entity: String, accountId: UUID, as type: T.Type) throws
    -> [T]
  {
    try rows(entity, accountId: accountId).map { row in
      guard let payload = row.value(forKey: "payload") as? Data else {
        throw WorkspaceError.missing
      }
      return try JSONDecoder().decode(type, from: payload)
    }
  }

  private struct Preferences: Codable {
    let policy: SyncPolicy
    let blockedAccounts: Set<UUID>
    let scratch: VoyageFields?
    let conflictReviews: [ConflictReview]?
    let pendingFileRemovals: [String]?
  }

  private func load(_ accountId: UUID) throws -> Workspace {
    guard let account = try decode("Account", accountId: accountId, as: LocalAccount.self).first
    else { throw WorkspaceError.missing }
    var workspace = Workspace(account: account)
    workspace.voyages = try decode("Voyage", accountId: accountId, as: LocalVoyage.self)
    workspace.photos = try decode("Photo", accountId: accountId, as: LocalPhoto.self)
    workspace.operations = try decode("Operation", accountId: accountId, as: OutboxOperation.self)
    workspace.workingCopies = try decode(
      "WorkingCopy", accountId: accountId, as: VoyageWorkingCopy.self)
    workspace.publicationIntents = try decode(
      "Publication", accountId: accountId, as: PublicationIntent.self)
    workspace.transfers = try decode("Transfer", accountId: accountId, as: BackgroundTransfer.self)
    if let preferences = try decode("Preferences", accountId: accountId, as: Preferences.self).first
    {
      workspace.policy = preferences.policy
      workspace.blockedAccounts = preferences.blockedAccounts
      workspace.scratch = preferences.scratch
      workspace.conflictReviews = preferences.conflictReviews
      workspace.pendingFileRemovals = preferences.pendingFileRemovals
    }
    return workspace
  }

  private func replace<T: Encodable & Identifiable>(_ entity: String, values: [T], accountId: UUID)
    throws where T.ID == UUID
  {
    let existing = try rows(entity, accountId: accountId)
    var byId = Dictionary(
      uniqueKeysWithValues: existing.map { ($0.value(forKey: "id") as! UUID, $0) })
    for value in values {
      let row =
        byId.removeValue(forKey: value.id)
        ?? NSEntityDescription.insertNewObject(forEntityName: entity, into: context)
      row.setValue(value.id, forKey: "id")
      row.setValue(accountId, forKey: "accountId")
      row.setValue(try JSONEncoder().encode(value), forKey: "payload")
    }
    for row in byId.values { context.delete(row) }
  }

  private func commit(_ workspace: Workspace) throws {
    let accountId = workspace.account.id
    try replace("Account", values: [workspace.account], accountId: accountId)
    try replace("Voyage", values: workspace.voyages, accountId: accountId)
    try replace("Photo", values: workspace.photos, accountId: accountId)
    try replace("Operation", values: workspace.operations, accountId: accountId)
    try replace("WorkingCopy", values: workspace.workingCopies, accountId: accountId)
    try replace("Publication", values: workspace.publicationIntents, accountId: accountId)
    try replace("Transfer", values: workspace.transfers, accountId: accountId)
    struct Row: Encodable, Identifiable {
      let id: UUID
      let policy: SyncPolicy
      let blockedAccounts: Set<UUID>
      let scratch: VoyageFields?
      let conflictReviews: [ConflictReview]?
      let pendingFileRemovals: [String]?
    }
    try replace(
      "Preferences",
      values: [
        Row(
          id: accountId, policy: workspace.policy, blockedAccounts: workspace.blockedAccounts,
          scratch: workspace.scratch, conflictReviews: workspace.conflictReviews,
          pendingFileRemovals: workspace.pendingFileRemovals)
      ], accountId: accountId)
    do { try context.save() } catch {
      context.rollback()
      throw error
    }
  }
}
