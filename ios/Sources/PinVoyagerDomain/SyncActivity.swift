import Foundation

public struct SyncActivity: Sendable {
  public enum Section: String, CaseIterable, Identifiable, Sendable {
    case needsAttention = "Needs attention"
    case inProgress = "In progress"
    case waiting = "Waiting"
    case completed = "Completed"
    public var id: Self { self }
  }
  public struct Row: Identifiable, Sendable {
    public var id: UUID? { voyageId }
    public let voyageId: UUID?
    public let operations: [OutboxOperation]
  }
  private let operations: [OutboxOperation]
  private let now: Date
  public init(operations: [OutboxOperation], now: Date = Date()) {
    self.operations = operations
    self.now = now
  }
  public func rows(for section: Section) -> [Row] {
    let matching = operations.filter { operation in
      switch (section, operation.state) {
      case (.needsAttention, .attention), (.needsAttention, .conflict),
        (.inProgress, .submitted), (.inProgress, .uncertain), (.waiting, .waiting):
        true
      case (.completed, .applied):
        operation.completedAt.map { $0 > now.addingTimeInterval(-7 * 86_400) } == true
      default: false
      }
    }
    return Dictionary(grouping: matching, by: \.voyageId).map {
      Row(voyageId: $0.key, operations: $0.value.sorted { $0.id.uuidString < $1.id.uuidString })
    }
    .sorted { ($0.voyageId?.uuidString ?? "") < ($1.voyageId?.uuidString ?? "") }
  }
}
