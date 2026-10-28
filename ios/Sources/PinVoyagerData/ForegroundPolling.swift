import Foundation

/// View tasks own this loop; cancellation stops waits and prevents another refresh.
public enum ForegroundPolling {
  @MainActor public static func run(
    refresh: @MainActor () async -> Bool,
    sleep: @MainActor (Duration) async throws -> Void = { try await Task.sleep(for: $0) }
  ) async throws {
    try await observe(refresh: { await refresh() ? true : nil }, sleep: sleep)
  }

  /// Return nil when work is finished. A meaningful status change resets the delay.
  @MainActor public static func observe<Snapshot: Equatable>(
    refresh: @MainActor () async -> Snapshot?,
    sleep: @MainActor (Duration) async throws -> Void = { try await Task.sleep(for: $0) }
  ) async throws {
    let delays: [Duration] = [.seconds(2), .seconds(5), .seconds(15), .seconds(30)]
    var attempt = 0
    var previous: Snapshot?
    while true {
      try Task.checkCancellation()
      guard let snapshot = await refresh() else { return }
      if snapshot != previous { attempt = 0 }
      previous = snapshot
      try await sleep(delays[attempt])
      attempt = min(attempt + 1, delays.count - 1)
    }
  }
}
