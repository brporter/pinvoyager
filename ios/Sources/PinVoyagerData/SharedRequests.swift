import Foundation

/// Share only concurrent work. Every later call starts a fresh authorization request.
actor SharedRequests<Key: Hashable & Sendable, Value: Sendable> {
  private struct Flight {
    let id: UUID
    let task: Task<Value, any Error>
  }
  private var flights: [Key: Flight] = [:]
  func value(for key: Key, operation: @escaping @Sendable () async throws -> Value) async throws
    -> Value
  {
    let flight: Flight
    if let active = flights[key] {
      flight = active
    } else {
      flight = Flight(id: UUID(), task: Task { try await operation() })
      flights[key] = flight
    }
    defer { if flights[key]?.id == flight.id { flights.removeValue(forKey: key) } }
    return try await flight.task.value
  }
}
