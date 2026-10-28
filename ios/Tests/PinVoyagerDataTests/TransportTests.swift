import Foundation
import Testing

@testable import PinVoyagerData

private final class ProtocolReplies: @unchecked Sendable {
  static let shared = ProtocolReplies()
  private let lock = NSLock()
  private var response: URLResponse?
  func set(_ response: URLResponse) {
    lock.lock()
    defer { lock.unlock() }
    self.response = response
  }
  func get() -> URLResponse {
    lock.lock()
    defer { lock.unlock() }
    return response!
  }
}

private final class ForegroundURLProtocol: URLProtocol, @unchecked Sendable {
  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
  override func startLoading() {
    client?.urlProtocol(
      self, didReceive: ProtocolReplies.shared.get(), cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: Data("response-body".utf8))
    client?.urlProtocolDidFinishLoading(self)
  }
  override func stopLoading() {}
}

@Test func foregroundSessionUsesHTTPStatusAndRejectsNonHTTPResponses() async throws {
  let configuration = URLSessionConfiguration.ephemeral
  configuration.protocolClasses = [ForegroundURLProtocol.self]
  let transport = URLSessionTransport(configuration: configuration)
  let url = URL(string: "https://pinvoyager.com/health")!
  ProtocolReplies.shared.set(
    HTTPURLResponse(
      url: url, statusCode: 200, httpVersion: "HTTP/1.1",
      headerFields: ["x-request-id": "reference"])!)
  let response = try await transport.send(URLRequest(url: url))
  #expect(response.status == 200)
  #expect(response.body == Data("response-body".utf8))
  #expect(response.headers["x-request-id"] == "reference")
  ProtocolReplies.shared.set(
    URLResponse(url: url, mimeType: "text/plain", expectedContentLength: 0, textEncodingName: nil))
  await #expect(throws: URLError.self) { try await transport.send(URLRequest(url: url)) }
}

@Test func foregroundRedirectDelegateNeverForwardsCredentials() {
  let session = URLSession(configuration: .ephemeral)
  let source = URL(string: "https://pinvoyager.com/auth")!
  let response = HTTPURLResponse(url: source, statusCode: 302, httpVersion: nil, headerFields: nil)!
  RefuseRedirects().urlSession(
    session, task: session.dataTask(with: source), willPerformHTTPRedirection: response,
    newRequest: URLRequest(url: URL(string: "https://evil.test")!)
  ) { request in
    #expect(request == nil)
  }
  session.invalidateAndCancel()
}
