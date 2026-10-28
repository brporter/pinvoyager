import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

actor TestPublicationNotifications: PublicationNotifications {
  var allowed = true
  var requested = 0
  var sent: [String: String] = [:]
  var existing: Set<String> = []
  var fail = false
  var deliveries = 0
  var afterSend: (@Sendable () async throws -> Void)?
  func setAfterSend(_ action: @escaping @Sendable () async throws -> Void) { afterSend = action }
  func requestPermission() -> Bool {
    requested += 1
    return allowed
  }
  func contains(id: String) -> Bool { existing.contains(id) }
  func send(id: String, title: String) async throws {
    if fail { throw URLError(.unknown) }
    sent[id] = title
    existing.insert(id)
    deliveries += 1
    try await afterSend?()
  }
  func remove(id: String) {
    existing.remove(id)
    sent.removeValue(forKey: id)
  }
  func allow(_ value: Bool) { allowed = value }
  func setFailure(_ value: Bool) { fail = value }
  func setExisting(_ id: String) { existing.insert(id) }
}

@Test func publicationNotificationsAreOptInAndRequireBothTerminalAndCurrentPublication()
  async throws
{
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.serverId = UUID()
  intent.status = .published
  state.publicationIntents = [intent]
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let notifications = TestPublicationNotifications()
  let notifier = PublicationNotifier(store: store, notifications: notifications)
  try await notifier.notifyCompleted(accountId: state.account.id)
  #expect(await notifications.requested == 0)
  #expect(await notifications.sent.isEmpty)
  try await notifier.setEnabled(true, accountId: state.account.id)
  try await notifier.notifyCompleted(accountId: state.account.id)
  #expect(await notifications.sent.isEmpty)
  try await store.update(accountId: state.account.id) { $0.voyages[0].confirmedPublished = true }
  try await notifier.notifyCompleted(accountId: state.account.id)
  try await notifier.notifyCompleted(accountId: state.account.id)
  #expect(await notifications.sent.count == 1)
  #expect(try await store.read(accountId: state.account.id).publicationIntents[0].notifiedAt != nil)
  try await notifier.setEnabled(false, accountId: state.account.id)
  #expect(try await store.read(accountId: state.account.id).policy.notifyOnPublication != true)
  #expect(await notifications.requested == 1)
}

@Test func deniedNotificationPermissionAndDeliveryFailuresKeepPublicationStateAndAllowRetry()
  async throws
{
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.status = .published
  state.publicationIntents = [intent]
  state.voyages[0].confirmedPublished = true
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let notifications = TestPublicationNotifications()
  let notifier = PublicationNotifier(store: store, notifications: notifications)
  await notifications.allow(false)
  await #expect(throws: ValidationError.self) {
    try await notifier.setEnabled(true, accountId: state.account.id)
  }
  #expect(try await store.read(accountId: state.account.id).policy.notifyOnPublication != true)
  await notifications.allow(true)
  try await notifier.setEnabled(true, accountId: state.account.id)
  await notifications.setFailure(true)
  await #expect(throws: URLError.self) {
    try await notifier.notifyCompleted(accountId: state.account.id)
  }
  #expect(
    try await store.read(accountId: state.account.id).publicationIntents[0].status == .published)
  #expect(try await store.read(accountId: state.account.id).publicationIntents[0].notifiedAt == nil)
  await notifications.setFailure(false)
  try await notifier.notifyCompleted(accountId: state.account.id)
  #expect(await notifications.sent.count == 1)
}

@Test func existingSystemNotificationsRecoverTheirCheckpointAndLockedAccountsStaySilent()
  async throws
{
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.status = .published
  state.publicationIntents = [intent]
  state.voyages[0].confirmedPublished = true
  state.policy.notifyOnPublication = true
  state.account.access = .locked
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let notifications = TestPublicationNotifications()
  let notifier = PublicationNotifier(store: store, notifications: notifications)
  try await notifier.notifyCompleted(accountId: state.account.id)
  await #expect(throws: WorkspaceError.locked) {
    try await notifier.setEnabled(true, accountId: state.account.id)
  }
  #expect(await notifications.requested == 0)
  try await store.update(accountId: state.account.id) { $0.account.access = .unlocked }
  await notifications.setExisting(
    PublicationNotifier.identifier(accountId: state.account.id, intentId: intent.id))
  try await notifier.notifyCompleted(accountId: state.account.id)
  #expect(await notifications.sent.isEmpty)
  #expect(try await store.read(accountId: state.account.id).publicationIntents[0].notifiedAt != nil)
}

@Test func signOutDuringNotificationDeliveryRemovesTheStaleNotification() async throws {
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.status = .published
  state.publicationIntents = [intent]
  state.voyages[0].confirmedPublished = true
  state.policy.notifyOnPublication = true
  let account = state.account.id
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let notifications = TestPublicationNotifications()
  await notifications.setAfterSend {
    try await LocalRepository(store: store).lock(accountId: account)
  }
  try await PublicationNotifier(store: store, notifications: notifications).notifyCompleted(
    accountId: account)
  #expect(await notifications.existing.isEmpty)
  #expect(await notifications.sent.isEmpty)
  #expect(try await store.read(accountId: account).publicationIntents[0].notifiedAt == nil)
}

@Test func concurrentPublicationChecksScheduleEachNotificationOnce() async throws {
  var state = workspaceWithPhotos(remote: true)
  var intent = try PublicationIntent.review(
    voyage: state.voyages[0], photos: state.photos, at: Date())
  intent.status = .published
  state.publicationIntents = [intent]
  state.voyages[0].confirmedPublished = true
  state.policy.notifyOnPublication = true
  let account = state.account.id
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let notifications = TestPublicationNotifications()
  let began = AsyncStream<Void>.makeStream()
  let release = AsyncStream<Void>.makeStream()
  await notifications.setAfterSend {
    began.continuation.yield(())
    for await _ in release.stream { break }
  }
  let notifier = PublicationNotifier(store: store, notifications: notifications)
  let first = Task { try await notifier.notifyCompleted(accountId: account) }
  for await _ in began.stream { break }
  try await notifier.notifyCompleted(accountId: account)
  release.continuation.yield(())
  try await first.value
  began.continuation.finish()
  release.continuation.finish()
  #expect(await notifications.deliveries == 1)
}
