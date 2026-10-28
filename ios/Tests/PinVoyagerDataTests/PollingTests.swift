import Foundation
import Testing

@testable import PinVoyagerData

@Test @MainActor func processingPollingSlowsDownAndStopsWhenTheLastPhotoIsReady() async throws {
  var passes = 0
  var delays: [Duration] = []
  try await ForegroundPolling.run(
    refresh: {
      passes += 1
      return passes < 6
    }, sleep: { delay in delays.append(delay) })
  #expect(passes == 6)
  #expect(delays == [.seconds(2), .seconds(5), .seconds(15), .seconds(30), .seconds(30)])
}

@Test @MainActor func canceledProcessingPollingStopsWithoutAnotherRequest() async throws {
  var passes = 0
  await #expect(throws: CancellationError.self) {
    try await ForegroundPolling.run(
      refresh: {
        passes += 1
        return true
      }, sleep: { _ in throw CancellationError() })
  }
  #expect(passes == 1)
}

@Test @MainActor func aChangedProcessingSnapshotRestartsTheShortPollingDelay() async throws {
  var snapshots: [String?] = [
    "three pending", "three pending", "two pending", "two pending", "two pending", nil,
  ]
  var delays: [Duration] = []
  try await ForegroundPolling.observe(
    refresh: { snapshots.removeFirst() }, sleep: { delays.append($0) })
  #expect(delays == [.seconds(2), .seconds(5), .seconds(2), .seconds(5), .seconds(15)])
}
