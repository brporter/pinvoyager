import Foundation
import PinVoyagerDomain

/// The same durable retry policy applies to foreground requests and system-managed uploads.
struct SynchronizationFailure: Sendable {
  let disposition: ResponseDisposition
  let message: String
  let requestId: String?
  let retryAfter: TimeInterval

  init(_ error: any Error) {
    let api = error as? APIError
    if let api {
      disposition = .classify(status: api.status)
    } else if let transport = error as? URLError,
      [
        .serverCertificateUntrusted, .serverCertificateHasUnknownRoot,
        .serverCertificateHasBadDate, .serverCertificateNotYetValid,
        .clientCertificateRejected, .clientCertificateRequired,
        .appTransportSecurityRequiresSecureConnection,
      ].contains(transport.code)
    {
      disposition = .attention
    } else if error is ValidationError || error is FileVaultError || error is OriginError {
      disposition = .attention
    } else {
      disposition = .reconcile
    }
    message = error.localizedDescription
    requestId = api?.requestId
    retryAfter = api?.retryAfter ?? 0
  }

  func apply(
    to state: inout Workspace, operationId: UUID, generation: Int, now: Date, randomUnit: Double,
    allowApplied: Bool = false
  ) {
    guard state.account.access == .unlocked, state.account.generation == generation,
      let index = state.operations.firstIndex(where: { $0.id == operationId }),
      state.operations[index].state != .canceled,
      allowApplied || state.operations[index].state != .applied
    else { return }
    state.operations[index].lastError = message
    state.operations[index].requestId = requestId
    switch disposition {
    case .signIn:
      state.account.access = .requiresSignIn
      state.account.generation += 1
      state.operations[index].state = .uncertain
    case .conflict, .destinationMissing:
      state.operations[index].state = .conflict
    case .attention:
      state.operations[index].state = .attention
    default:
      state.operations[index].attempts += 1
      state.operations[index].state =
        state.operations[index].attempts >= 8 ? .attention : .uncertain
      state.operations[index].nextAttempt = now.addingTimeInterval(
        RetrySchedule.delay(
          attempt: state.operations[index].attempts, randomUnit: randomUnit, retryAfter: retryAfter)
      )
    }
    if state.operations[index].state == .attention,
      case .upload(let photoId, _) = state.operations[index].kind,
      let photo = state.photos.firstIndex(where: { $0.id == photoId && !$0.deleted })
    {
      state.photos[photo].transfer = .needsAttention
    }
  }
}
