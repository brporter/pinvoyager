import Foundation
import Observation
import PinVoyagerData
import PinVoyagerDomain

@MainActor @Observable
final class PhotoImportPresentation {
  private let service: PhotoImportService
  private(set) var preparing = false
  private(set) var progress = ""
  private(set) var failures: [String] = []
  init(service: PhotoImportService) { self.service = service }

  func stop() async { await service.cancel() }

  func importPhotos(
    _ sources: [any PhotoAcquiring], accountId: UUID, voyageId: UUID,
    maxBytes: Int64, publicConsent: Bool, cellularException: Bool = false,
    didChange: @escaping @MainActor @Sendable () async -> Void
  ) async {
    guard !preparing, !sources.isEmpty else { return }
    preparing = true
    defer { preparing = false }
    do {
      _ = try await service.importBatch(
        sources, accountId: accountId, voyageId: voyageId,
        maxBytes: maxBytes, publicConsent: publicConsent, cellularException: cellularException
      ) { [weak self] event in
        await self?.record(event, didChange: didChange)
      }
    } catch { failures.append(error.localizedDescription) }
  }

  private func record(_ event: PhotoImportEvent, didChange: @MainActor () async -> Void) async {
    switch event {
    case .preparing(let index, let total): progress = "Saving \(index) of \(total) on this iPhone"
    case .saved: break
    case .failed(_, let message): failures.append(message)
    }
    await didChange()
  }
}
