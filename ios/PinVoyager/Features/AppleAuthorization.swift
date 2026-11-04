import AuthenticationServices
import Foundation
import PinVoyagerData

struct AppleAuthorization: Sendable {
  let code: String
  let state: String?
}

@MainActor
protocol AppleAuthorizing {
  func authorize(nonce: String, state: String) async throws -> AppleAuthorization
}

@MainActor
protocol AppleAuthorizationControlling: AnyObject {
  var delegate: (any ASAuthorizationControllerDelegate)? { get set }
  var presentationContextProvider: (any ASAuthorizationControllerPresentationContextProviding)? {
    get set
  }
  func performRequests()
}

extension ASAuthorizationController: AppleAuthorizationControlling {}

@MainActor
final class SystemAppleAuthorization: NSObject, AppleAuthorizing,
  ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding
{
  typealias Factory = (ASAuthorizationAppleIDRequest) -> any AppleAuthorizationControlling
  private let anchor: () -> ASPresentationAnchor
  private let factory: Factory
  private var controller: (any AppleAuthorizationControlling)?
  private var continuation: CheckedContinuation<AppleAuthorization, any Error>?

  init(
    anchor: @escaping () -> ASPresentationAnchor,
    factory: @escaping Factory = { ASAuthorizationController(authorizationRequests: [$0]) }
  ) {
    self.anchor = anchor
    self.factory = factory
  }

  func authorize(nonce: String, state: String) async throws -> AppleAuthorization {
    guard continuation == nil else { throw AuthenticationError.invalidAttempt }
    return try await withCheckedThrowingContinuation { continuation in
      let request = ASAuthorizationAppleIDProvider().createRequest()
      request.nonce = nonce
      request.state = state
      // PinVoyager uses the verified subject and asks for no name or email scopes.
      request.requestedScopes = []
      let controller = factory(request)
      controller.delegate = self
      controller.presentationContextProvider = self
      self.controller = controller
      self.continuation = continuation
      controller.performRequests()
    }
  }

  func authorizationController(
    controller: ASAuthorizationController,
    didCompleteWithAuthorization authorization: ASAuthorization
  ) {
    let credential = authorization.credential as? ASAuthorizationAppleIDCredential
    finish(Self.credential(code: credential?.authorizationCode, state: credential?.state))
  }

  static func credential(code: Data?, state: String?) -> Result<AppleAuthorization, any Error> {
    guard let code, let value = String(data: code, encoding: .utf8), !value.isEmpty else {
      return .failure(AuthenticationError.invalidAttempt)
    }
    return .success(AppleAuthorization(code: value, state: state))
  }

  func authorizationController(
    controller: ASAuthorizationController,
    didCompleteWithError error: any Error
  ) {
    finish(.failure(error))
  }

  private func finish(_ result: Result<AppleAuthorization, any Error>) {
    let continuation = self.continuation
    self.continuation = nil
    controller = nil
    continuation?.resume(with: result)
  }

  func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
    anchor()
  }
}
