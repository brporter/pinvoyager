import Foundation
import Testing

@testable import PinVoyagerDomain

@Test func syncActivityOrderDoesNotDependOnTheStoresFetchOrder() throws {
  let account = UUID()
  let voyage = UUID()
  let ids = try [3, 1, 2].map {
    try #require(UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", $0)))
  }
  let operations = ids.map { id in
    OutboxOperation(
      id: id, accountId: account, generation: 0, voyageId: voyage, kind: .deleteVoyage)
  }
  let expected = ids.sorted { $0.uuidString < $1.uuidString }
  for snapshot in [operations, operations.reversed()] {
    let rows = SyncActivity(operations: Array(snapshot)).rows(for: .waiting)
    #expect(rows.first?.operations.map(\.id) == expected)
  }
}

@Test func syncActivityGroupsVoyagesAndExpiresOnlyCompletedPresentationRows() {
  let account = UUID()
  let voyage = UUID()
  let other = UUID()
  let now = Date(timeIntervalSince1970: 2_000_000)
  func operation(_ id: UUID?, _ status: OperationState) -> OutboxOperation {
    var operation = OutboxOperation(
      accountId: account, generation: 0, voyageId: id, kind: .deleteVoyage)
    operation.state = status
    return operation
  }
  let waiting = operation(voyage, .waiting)
  let attention = operation(voyage, .attention)
  let conflict = operation(voyage, .conflict)
  let active = operation(other, .uncertain)
  let submitted = operation(other, .submitted)
  var recent = operation(nil, .applied)
  recent.completedAt = now.addingTimeInterval(-100)
  var old = operation(other, .applied)
  old.completedAt = now.addingTimeInterval(-7 * 86_400)
  let unknown = operation(other, .applied)
  let canceled = operation(voyage, .canceled)
  let operations = [
    waiting, attention, conflict, active, submitted, recent, old, unknown, canceled,
  ]
  let activity = SyncActivity(operations: operations, now: now)
  #expect(activity.rows(for: .needsAttention).count == 1)
  #expect(
    Set(activity.rows(for: .needsAttention)[0].operations.map(\.id)) == [attention.id, conflict.id])
  #expect(activity.rows(for: .inProgress)[0].voyageId == other)
  #expect(activity.rows(for: .waiting)[0].operations == [waiting])
  #expect(activity.rows(for: .completed)[0].operations == [recent])
  #expect(activity.rows(for: .completed)[0].voyageId == nil)
  #expect(operations.count == 9)
}
