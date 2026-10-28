import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

func temporaryDirectory() throws -> URL {
  let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
  return url
}

func unlockedWorkspace() -> Workspace {
  Workspace(account: LocalAccount(profile: UserProfile(id: UUID(), displayName: "alice")))
}

@Test func sqliteRoundTripAndAccountIsolation() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("workspace.sqlite")
  let store = try CoreDataWorkspaceStore(url: url)
  let workspace = unlockedWorkspace()
  let other = unlockedWorkspace()
  try await store.insert(workspace)
  try await store.insert(other)
  let repository = LocalRepository(store: store)
  let id = try await repository.createVoyage(
    accountId: workspace.account.id, fields: VoyageFields(title: "Ice", description: "Winter"))
  let reopened = try CoreDataWorkspaceStore(url: url)
  let saved = try await reopened.read(accountId: workspace.account.id)
  #expect(saved.voyages.first?.id == id)
  #expect(saved.operations.count == 1)
  #expect(saved.operations.first?.state == .waiting)
  #expect(try await reopened.read(accountId: other.account.id).voyages.isEmpty)
  #expect(saved.voyages.first?.confirmedPublished == false)
}

@Test func failedTransactionDoesNotPartiallySave() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  await #expect(throws: ValidationError.self) {
    try await store.update(accountId: workspace.account.id) { state in
      state.voyages.append(
        LocalVoyage(ownerId: state.account.id, fields: VoyageFields(title: "x", description: "y")))
      throw ValidationError(field: "test", message: "roll back")
    }
  }
  #expect(try await store.read(accountId: workspace.account.id).voyages.isEmpty)
}

@Test func concurrentLocalSavesPreserveEveryOperation() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  let repository = LocalRepository(store: store)
  try await withThrowingTaskGroup(of: Void.self) { group in
    for index in 0..<20 {
      group.addTask {
        _ = try await repository.createVoyage(
          accountId: workspace.account.id,
          fields: VoyageFields(title: "Trip \(index)", description: "Saved"))
      }
    }
    try await group.waitForAll()
  }
  let saved = try await store.read(accountId: workspace.account.id)
  #expect(saved.voyages.count == 20)
  #expect(Set(saved.operations.map(\.id)).count == 20)
}

@Test func workingCopyCancelAndCommitAreSeparate() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  let repository = LocalRepository(store: store)
  let id = try await repository.createVoyage(
    accountId: workspace.account.id, fields: VoyageFields(title: "Before", description: "Saved"))
  let changed = VoyageFields(title: "After", description: "Edited")
  try await repository.saveWorkingCopy(
    accountId: workspace.account.id, voyageId: id, fields: changed)
  #expect(
    try await store.read(accountId: workspace.account.id).voyages.first?.fields.title == "Before")
  try await repository.discardWorkingCopy(accountId: workspace.account.id, voyageId: id)
  #expect(try await store.read(accountId: workspace.account.id).workingCopies.isEmpty)
  try await repository.editVoyage(accountId: workspace.account.id, voyageId: id, fields: changed)
  let saved = try await store.read(accountId: workspace.account.id)
  #expect(saved.voyages.first?.fields.title == "After")
  #expect(saved.operations.count == 1)  // Coalesce the never-submitted creation.
}

@Test func signOutLocksWorkspaceAndChangesGeneration() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let workspace = unlockedWorkspace()
  try await store.insert(workspace)
  let repository = LocalRepository(store: store)
  try await repository.lock(accountId: workspace.account.id)
  let saved = try await store.read(accountId: workspace.account.id)
  #expect(saved.account.access == .locked)
  #expect(saved.account.generation == 1)
  await #expect(throws: WorkspaceError.self) {
    try await repository.createVoyage(
      accountId: workspace.account.id, fields: VoyageFields(title: "x", description: "y"))
  }
}

@Test func scratchDraftsAreDurableAndAccountScoped() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let first = unlockedWorkspace()
  let second = unlockedWorkspace()
  try await store.insert(first)
  try await store.insert(second)
  let repository = LocalRepository(store: store)
  let fields = VoyageFields(title: "Unfinished", description: "")
  try await repository.saveScratch(accountId: first.account.id, fields: fields)
  #expect(try await store.read(accountId: first.account.id).scratch == fields)
  #expect(try await store.read(accountId: second.account.id).scratch == nil)
  #expect(try await store.read(accountId: first.account.id).operations.isEmpty)
  try await repository.saveScratch(accountId: first.account.id, fields: nil)
  #expect(try await store.read(accountId: first.account.id).scratch == nil)
}

@Test func incompleteImportSurvivesWithoutPretendingToBeSaved() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = workspaceWithPhotos()
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  let id = try await repository.beginImport(
    accountId: state.account.id, voyageId: state.voyages[0].id, filename: "Photo from iCloud")
  let saved = try await store.read(accountId: state.account.id)
  let acquiring = try #require(saved.photos.first { $0.id == id })
  #expect(acquiring.transfer == .acquiring)
  #expect(!acquiring.isDurable)
  #expect(saved.operations.isEmpty)
  try await repository.recoverInterruptedImports(accountId: state.account.id)
  #expect(
    try await store.read(accountId: state.account.id).photos.first { $0.id == id }?.transfer
      == .needsSource)
}

@Test func networkPolicyChangeInvalidatesOldTransferCallbacks() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = unlockedWorkspace()
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  var policy = state.policy
  policy.wifiOnly = false
  try await repository.setPolicy(accountId: state.account.id, policy: policy)
  #expect(try await store.read(accountId: state.account.id).account.generation == 1)
  try await repository.setPolicy(accountId: state.account.id, policy: policy)
  #expect(try await store.read(accountId: state.account.id).account.generation == 1)
}

@Test func fileVaultCopiesBeforeRecordingAndRejectsTraversal() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let source = directory.appendingPathComponent("source.jpg")
  try Data([0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]).write(to: source)
  let vault = try FileVault(root: directory.appendingPathComponent("private"))
  let accountId = UUID()
  let saved = try await vault.importFile(
    source, accountId: accountId, photoId: UUID(), maxBytes: 100)
  #expect(saved.byteCount == 7)
  #expect(saved.mediaType == "image/jpeg")
  #expect(saved.contentHash.count == 64)
  #expect(
    try await vault.read(saved.relativePath, accountId: accountId) == Data(contentsOf: source))
  await #expect(throws: FileVaultError.self) {
    try await vault.read("../source.jpg", accountId: accountId)
  }
  await #expect(throws: FileVaultError.self) {
    try await vault.importFile(source, accountId: accountId, photoId: UUID(), maxBytes: 2)
  }
  let invalid = directory.appendingPathComponent("invalid")
  try Data("not an image".utf8).write(to: invalid)
  await #expect(throws: FileVaultError.self) {
    try await vault.importFile(invalid, accountId: accountId, photoId: UUID(), maxBytes: 100)
  }
}

@Test func multipartIsFileBackedAndPreservesExactBytes() async throws {
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let source = directory.appendingPathComponent("source")
  let bytes = Data([0, 1, 2, 0xff])
  try bytes.write(to: source)
  let result = try MultipartBody.write(
    source: source, destination: directory.appendingPathComponent("body"), mediaType: "image/jpeg",
    boundary: "safe-boundary", maxBytes: 1_000)
  let body = try Data(contentsOf: result.url)
  #expect(body.count == result.byteCount)
  #expect(body.range(of: bytes) != nil)
  #expect(String(decoding: body, as: UTF8.self).contains("name=\"file\""))
  #expect(String(decoding: body, as: UTF8.self).hasSuffix("\r\n--safe-boundary--\r\n"))
  #expect(throws: FileVaultError.self) {
    try MultipartBody.write(
      source: source, destination: directory.appendingPathComponent("too-large"),
      mediaType: "image/jpeg", boundary: "safe", maxBytes: 2)
  }
}

@Test func photoRemovalCanBeUndoneOnlyBeforeItsRequestIsSubmitted() async throws {
  for submitted in [false, true] {
    let state = workspaceWithPhotos()
    let store = try CoreDataWorkspaceStore(url: nil)
    try await store.insert(state)
    let repository = LocalRepository(store: store)
    let photo = state.photos[0]
    try await repository.removePhoto(accountId: state.account.id, photoId: photo.id)
    if submitted {
      try await store.update(accountId: state.account.id) { state in
        state.operations[0].request = MutationRequest(
          method: "DELETE", path: "/api/voyages/test/uploads/test")
        state.operations[0].state = .submitted
      }
      await #expect(throws: ValidationError.self) {
        try await repository.undoPhotoRemoval(accountId: state.account.id, photoId: photo.id)
      }
    } else {
      try await repository.undoPhotoRemoval(accountId: state.account.id, photoId: photo.id)
    }
    let saved = try await store.read(accountId: state.account.id)
    #expect(saved.photos.first { $0.id == photo.id }?.deleted == submitted)
    #expect(saved.photos.first { $0.id == photo.id }?.source == photo.source)
    #expect(saved.photos.first { $0.id == photo.id }?.uploadKey == photo.uploadKey)
    #expect(saved.operations[0].state == (submitted ? .submitted : .canceled))
  }
}

@Test func acquiringAnotherPhotoInvalidatesUnsubmittedPublicationConsentImmediately() async throws {
  let state = workspaceWithPhotos()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  _ = try await repository.queuePublication(
    accountId: state.account.id, voyageId: state.voyages[0].id)
  _ = try await repository.beginImport(
    accountId: state.account.id, voyageId: state.voyages[0].id, filename: "Still in iCloud")
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.publicationIntents[0].status == .needsReview)
  #expect(
    saved.operations.allSatisfy {
      if case .publish = $0.kind { return $0.state == .canceled }
      return true
    })
}

@Test func reportsExposeThePersistedOperationIdentityForReceiptStatus() async throws {
  let store = try CoreDataWorkspaceStore(url: nil)
  let state = unlockedWorkspace()
  try await store.insert(state)
  let id = try await LocalRepository(store: store).report(
    accountId: state.account.id, target: "voyage:\(UUID())", reason: "spam",
    details: "Please review")
  let saved = try await store.read(accountId: state.account.id)
  #expect(saved.operations.first?.id == id)
}
