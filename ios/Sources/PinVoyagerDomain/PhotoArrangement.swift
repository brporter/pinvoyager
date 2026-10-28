import Foundation

/// Stages a complete order without changing the saved voyage until the user commits it.
public struct PhotoArrangement: Equatable, Sendable {
  public private(set) var ids: [UUID]
  public private(set) var selected: Set<UUID> = []
  public init(ids: [UUID]) { self.ids = ids }
  public mutating func reconcile(_ available: [UUID]) {
    let members = Set(available)
    ids.removeAll { !members.contains($0) }
    let existing = Set(ids)
    ids.append(contentsOf: available.filter { !existing.contains($0) })
    selected.formIntersection(members)
  }
  public mutating func reset(_ ids: [UUID]) { self = Self(ids: ids) }
  public mutating func toggleSelection(_ id: UUID) {
    guard ids.contains(id) else { return }
    if selected.contains(id) { selected.remove(id) } else { selected.insert(id) }
  }
  public mutating func move(_ id: UUID, by distance: Int) {
    guard let index = ids.firstIndex(of: id) else { return }
    let destination = index + distance
    guard ids.indices.contains(destination) else { return }
    ids.remove(at: index)
    ids.insert(id, at: destination)
  }
  public mutating func setOrder(_ value: [UUID]) {
    guard value.count == ids.count, Set(value) == Set(ids) else { return }
    ids = value
  }
}
