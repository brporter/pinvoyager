import Foundation
import Testing

@testable import PinVoyagerDomain

@Test func photoArrangementKeepsMovesAndSelectionWhileAppendingArrivalsAndRemovingDepartures() {
  let a = UUID()
  let b = UUID()
  let c = UUID()
  let added = UUID()
  var arrangement = PhotoArrangement(ids: [a, b, c])
  arrangement.move(b, by: -1)
  #expect(arrangement.ids == [b, a, c])
  arrangement.toggleSelection(a)
  arrangement.toggleSelection(c)
  arrangement.reconcile([a, b, added])
  #expect(arrangement.ids == [b, a, added])
  #expect(arrangement.selected == [a])
  arrangement.toggleSelection(a)
  arrangement.toggleSelection(UUID())
  #expect(arrangement.selected.isEmpty)
  arrangement.move(b, by: -1)
  arrangement.move(added, by: 1)
  arrangement.move(UUID(), by: 1)
  #expect(arrangement.ids == [b, a, added])
  arrangement.reset([a, b, added])
  #expect(arrangement.ids == [a, b, added])
  #expect(arrangement.selected.isEmpty)
}

@Test func photoArrangementSupportsCompleteDragOrdersAndIgnoresInvalidPermutations() {
  let ids = [UUID(), UUID(), UUID()]
  var arrangement = PhotoArrangement(ids: ids)
  arrangement.setOrder(Array(ids.reversed()))
  #expect(arrangement.ids == Array(ids.reversed()))
  arrangement.setOrder([ids[0], ids[0], ids[2]])
  arrangement.setOrder([ids[0]])
  #expect(arrangement.ids == Array(ids.reversed()))
}
