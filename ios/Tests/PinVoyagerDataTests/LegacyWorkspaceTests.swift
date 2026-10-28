import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func aVersionOnePendingRequestReopensWithoutNewOptionalFieldsAndReconcilesBeforeReplay()
  async throws
{
  let legacy = Data(
    #"""
    {"account":{"profile":{"id":"00000000-0000-0000-0000-000000000001","displayName":"traveler"},"generation":0,"access":"unlocked","verifiedAt":0},
     "voyages":[{"id":"00000000-0000-0000-0000-000000000002","ownerId":"00000000-0000-0000-0000-000000000001","fields":{"title":"Legacy draft","description":"Saved before the upgrade","granularity":"km1"},"localSequence":0,"confirmedPublished":false,"deleted":false,"updatedAt":0}],
     "photos":[],"operations":[{"id":"00000000-0000-0000-0000-000000000003","accountId":"00000000-0000-0000-0000-000000000001","generation":0,"voyageId":"00000000-0000-0000-0000-000000000002","kind":{"create":{"_0":{"title":"Legacy draft","description":"Saved before the upgrade","granularity":"km1"}}},"dependencies":[],"state":"uncertain","request":{"method":"POST","path":"/api/voyages","body":"eyJ0aXRsZSI6IkxlZ2FjeSBkcmFmdCIsImRlc2NyaXB0aW9uIjoiU2F2ZWQgYmVmb3JlIHRoZSB1cGdyYWRlIiwiZ3JhbnVsYXJpdHkiOiJrbTEifQ==","headers":{}},"attempts":1,"nextAttempt":0}],
     "workingCopies":[],"publicationIntents":[],"transfers":[],"blockedAccounts":[],"policy":{"wifiOnly":true,"allowsConstrained":false,"backgroundPreparation":true,"keepOriginals":true,"paused":false}}
    """#.utf8)
  let state = try JSONDecoder().decode(Workspace.self, from: legacy)
  let directory = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: directory) }
  let url = directory.appendingPathComponent("legacy.sqlite")
  let store = try CoreDataWorkspaceStore(url: url)
  try await store.insert(state)
  let reopened = try CoreDataWorkspaceStore(url: url)
  let restored = try await reopened.read(accountId: state.account.id)
  #expect(restored.operations[0].request == state.operations[0].request)
  #expect(restored.operations[0].receiptRejected == nil)
  #expect(restored.operations[0].completedAt == nil)
  #expect(restored.voyages[0].deletionUndo == nil)
  #expect(restored.voyages[0].uploadsPaused == nil)
  #expect(restored.pendingFileRemovals == nil)
  let remote = StubSyncRemote()
  let serverId = UUID()
  await remote.setReceipt(.applied(targetId: serverId, revision: "1"), id: state.operations[0].id)
  try await SyncCoordinator(store: reopened, remote: remote).run(
    accountId: state.account.id, network: .wifi(constrained: false))
  #expect(await remote.submissions.isEmpty)
  let settled = try await reopened.read(accountId: state.account.id)
  #expect(settled.voyages[0].serverId == serverId)
  #expect(settled.operations[0].request == state.operations[0].request)
}
