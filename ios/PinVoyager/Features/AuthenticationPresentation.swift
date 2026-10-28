import AuthenticationServices
import Observation
import PinVoyagerData
import PinVoyagerDomain

@MainActor protocol BrowserAuthenticating {
  func open(_ url: URL, ephemeral: Bool) async throws -> URL
}

@MainActor @Observable final class AuthenticationPresentation {
  private let authentication: NativeAuthentication
  private let browser: any BrowserAuthenticating
  private(set) var busy = false
  private(set) var error: String?
  init(authentication: NativeAuthentication, browser: any BrowserAuthenticating) {
    self.authentication = authentication
    self.browser = browser
  }
  func signIn(
    provider: String, linkingAccountId: UUID? = nil, ephemeral: Bool,
    accept: @MainActor (SessionExchange) async throws -> Void
  ) async -> Bool {
    await perform(
      ephemeral: ephemeral,
      begin: {
        try await authentication.begin(provider: provider, linkingAccountId: linkingAccountId)
      },
      finish: { callback, pending in
        try await accept(authentication.complete(callback, pending: pending))
      })
  }
  func reauthenticate(
    provider: String, accountId: UUID,
    accept: @MainActor (DeletionProof) async throws -> Void
  ) async -> Bool {
    await perform(
      ephemeral: false,
      begin: {
        try await authentication.beginReauthentication(provider: provider, accountId: accountId)
      },
      finish: { callback, pending in
        try await accept(authentication.completeReauthentication(callback, pending: pending))
      })
  }
  private func perform(
    ephemeral: Bool, begin: () async throws -> PendingAuthentication,
    finish: (URL, PendingAuthentication) async throws -> Void
  ) async -> Bool {
    guard !busy else { return false }
    busy = true
    error = nil
    defer { busy = false }
    var pending: PendingAuthentication?
    do {
      let attempt = try await begin()
      pending = attempt
      let callback = try await browser.open(attempt.browserStartUrl, ephemeral: ephemeral)
      try await finish(callback, attempt)
      return true
    } catch let failure as ASWebAuthenticationSessionError where failure.code == .canceledLogin {
      // Closing the system browser returns to the current screen without an error alert.
    } catch AuthenticationError.canceled {
    } catch { self.error = error.localizedDescription }
    if let pending { await authentication.cancel(pending) }
    return false
  }
}

@MainActor protocol AuthenticationBrowserSession: AnyObject {
  var prefersEphemeralWebBrowserSession: Bool { get set }
  func start() -> Bool
}

extension ASWebAuthenticationSession: AuthenticationBrowserSession {}

private final class BrowserCallback: @unchecked Sendable {
  private let lock = NSLock()
  private var continuation: CheckedContinuation<URL, any Error>?
  init(_ continuation: CheckedContinuation<URL, any Error>) { self.continuation = continuation }
  func finish(_ result: Result<URL, any Error>) {
    let continuation = lock.withLock {
      let saved = self.continuation
      self.continuation = nil
      return saved
    }
    continuation?.resume(with: result)
  }
}

@MainActor
final class BrowserAuthentication: NSObject, BrowserAuthenticating,
  ASWebAuthenticationPresentationContextProviding
{
  typealias Factory = (
    URL, any ASWebAuthenticationPresentationContextProviding,
    @escaping @Sendable (URL?, (any Error)?) -> Void
  ) -> any AuthenticationBrowserSession
  private let anchor: () -> ASPresentationAnchor
  private let factory: Factory
  private var session: (any AuthenticationBrowserSession)?
  init(
    anchor: @escaping () -> ASPresentationAnchor,
    factory: @escaping Factory = { url, provider, complete in
      let session = ASWebAuthenticationSession(
        url: url, callbackURLScheme: "pinvoyager", completionHandler: complete)
      session.presentationContextProvider = provider
      return session
    }
  ) {
    self.anchor = anchor
    self.factory = factory
  }
  func open(_ url: URL, ephemeral: Bool) async throws -> URL {
    guard session == nil else { throw AuthenticationError.invalidAttempt }
    defer { session = nil }
    return try await withCheckedThrowingContinuation { continuation in
      let callback = BrowserCallback(continuation)
      let session = factory(url, self) { url, error in
        if let url {
          callback.finish(.success(url))
        } else {
          callback.finish(.failure(error ?? AuthenticationError.canceled))
        }
      }
      session.prefersEphemeralWebBrowserSession = ephemeral
      self.session = session
      if !session.start() { callback.finish(.failure(AuthenticationError.invalidAttempt)) }
    }
  }
  func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    anchor()
  }
}
