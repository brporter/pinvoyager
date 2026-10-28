import Foundation
import Testing

@testable import PinVoyagerData

@Test func temporaryCacheEvictsLeastRecentlyUsedFilesAndPersistsAcrossReopening() async throws {
  let root = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: root) }
  let cache = try TemporaryImageCache(directory: root, capacity: 300)
  let bytes = Data(repeating: 65, count: 80)
  try await cache.store(bytes, etag: "one", key: "one", now: Date(timeIntervalSince1970: 1))
  try await cache.store(bytes, etag: "two", key: "two", now: Date(timeIntervalSince1970: 2))
  #expect(try await cache.lookup("one", now: Date(timeIntervalSince1970: 3))?.data == bytes)
  try await cache.store(bytes, etag: "three", key: "three", now: Date(timeIntervalSince1970: 4))
  #expect(try await cache.lookup("two") == nil)
  #expect(try await cache.lookup("one")?.etag == "one")
  #expect(try await cache.byteCount() <= 300)
  let reopened = try TemporaryImageCache(directory: root, capacity: 300)
  #expect(try await reopened.lookup("three")?.etag == "three")
  try await cache.store(Data(repeating: 0, count: 1000), etag: "large", key: "large")
  #expect(try await cache.lookup("large") == nil)
  try await cache.clear()
  #expect(try await cache.byteCount() == 0)
}

@Test func cachedImageReuseAlwaysRequiresFreshOnlineAuthorizationAndAccountIsolation() async throws
{
  let root = try temporaryDirectory()
  defer { try? FileManager.default.removeItem(at: root) }
  let cache = try TemporaryImageCache(directory: root)
  let bytes = Data("webp variant".utf8)
  let transport = StubHTTP([
    HTTPResponse(status: 200, headers: ["ETag": "variant"], body: bytes),
    HTTPResponse(status: 304, body: Data()),
    HTTPResponse(status: 404, body: Data()),
    HTTPResponse(status: 200, body: bytes),
    HTTPResponse(status: 200, headers: ["ETag": "renewed"], body: bytes),
  ])
  let credentials = TestCredentials()
  let account = UUID()
  await credentials.save(SessionCredential(accountId: account, token: "private"))
  let client = APIClient(
    origin: try testOrigin(), transport: transport, credentials: credentials, imageCache: cache)
  let path = "/api/photos/\(UUID())/thumb"
  #expect(try await client.image(path) == bytes)
  #expect(try await client.image(path) == bytes)
  await #expect(throws: APIError.self) { try await client.image(path) }
  #expect(try await client.image(path, accountId: account) == bytes)
  #expect(try await client.image(path) == bytes)
  let requests = await transport.requests
  #expect(requests.count == 5)
  #expect(requests[1].value(forHTTPHeaderField: "If-None-Match") == "variant")
  #expect(requests[3].value(forHTTPHeaderField: "If-None-Match") == nil)
  #expect(requests[3].value(forHTTPHeaderField: "Authorization") == "Bearer private")
  #expect(requests[4].value(forHTTPHeaderField: "If-None-Match") == nil)
  let offline = APIClient(
    origin: try testOrigin(), transport: FailingTransport(), credentials: credentials,
    imageCache: cache)
  await #expect(throws: URLError.self) { try await offline.image(path) }
}

private struct FailingTransport: HTTPTransport {
  func send(_ request: URLRequest) throws -> HTTPResponse {
    throw URLError(.notConnectedToInternet)
  }
}
