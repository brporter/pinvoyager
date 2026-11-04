import AuthenticationServices
import Observation
import PinVoyagerData
import PinVoyagerDomain

@MainActor
protocol BrowserAuthenticating {
  func open(_ url: URL, ephemeral: Bool) async throws -> URL
}

@MainActor
@Observable
final class AuthenticationPresentation {
  private let authentication: NativeAuthentication
  private let browser: any BrowserAuthenticating
  private let apple: (any AppleAuthorizing)?
  private(set) var busy = false
  private(set) var error: String?

  init(
    authentication: NativeAuthentication,
    browser: any BrowserAuthenticating,
    apple: (any AppleAuthorizing)? = nil
  ) {
    self.authentication = authentication
    self.browser = browser
    self.apple = apple
  }

  func signIn(
    provider: String,
    linkingAccountId: UUID? = nil,
    ephemeral: Bool,
    accept: @MainActor (SessionExchange) async throws -> Void
  ) async -> Bool {
    if provider == "apple" {
      return await perform(
        begin: { try await authentication.beginApple(linkingAccountId: linkingAccountId) },
        complete: { pending in
          guard let apple else { throw AuthenticationError.invalidAttempt }
          let credential = try await apple.authorize(nonce: pending.nonce, state: pending.state)
          try await accept(
            authentication.completeApple(
              code: credential.code,
              state: credential.state,
              pending: pending
            )
          )
        },
        cancel: { await authentication.cancel($0) }
      )
    }
    return await perform(
      begin: {
        try await authentication.begin(provider: provider, linkingAccountId: linkingAccountId)
      },
      complete: { pending in
        let callback = try await browser.open(pending.browserStartUrl, ephemeral: ephemeral)
        try await accept(authentication.complete(callback, pending: pending))
      },
      cancel: { await authentication.cancel($0) }
    )
  }

  func reauthenticate(
    provider: String,
    accountId: UUID,
    accept: @MainActor (DeletionProof) async throws -> Void
  ) async -> Bool {
    if provider == "apple" {
      return await perform(
        begin: { try await authentication.beginApple(deletionAccountId: accountId) },
        complete: { pending in
          guard let apple else { throw AuthenticationError.invalidAttempt }
          let credential = try await apple.authorize(nonce: pending.nonce, state: pending.state)
          try await accept(
            authentication.completeAppleReauthentication(
              code: credential.code,
              state: credential.state,
              pending: pending
            )
          )
        },
        cancel: { await authentication.cancel($0) }
      )
    }
    return await perform(
      begin: {
        try await authentication.beginReauthentication(provider: provider, accountId: accountId)
      },
      complete: { pending in
        let callback = try await browser.open(pending.browserStartUrl, ephemeral: false)
        try await accept(authentication.completeReauthentication(callback, pending: pending))
      },
      cancel: { await authentication.cancel($0) }
    )
  }

  private func perform<Pending>(
    begin: () async throws -> Pending,
    complete: (Pending) async throws -> Void,
    cancel: (Pending) async -> Void
  ) async -> Bool {
    guard !busy else { return false }
    busy = true
    error = nil
    defer { busy = false }
    var pending: Pending?
    do {
      let attempt = try await begin()
      pending = attempt
      try await complete(attempt)
      return true
    } catch let failure as ASWebAuthenticationSessionError where failure.code == .canceledLogin {
    } catch let failure as ASAuthorizationError where failure.code == .canceled {
    } catch AuthenticationError.canceled {
    } catch { self.error = error.localizedDescription }
    if let pending { await cancel(pending) }
    return false
  }
}

@MainActor
protocol AuthenticationBrowserSession: AnyObject {
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
final class BrowserAuthentication: NSObject,
  BrowserAuthenticating,
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
        url: url,
        callbackURLScheme: "pinvoyager",
        completionHandler: complete
      )
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
