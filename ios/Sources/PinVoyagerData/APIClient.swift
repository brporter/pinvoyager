import Foundation
import PinVoyagerDomain

public struct HTTPResponse: Sendable {
  public let status: Int
  public let headers: [String: String]
  public let body: Data
  public init(status: Int, headers: [String: String] = [:], body: Data) {
    self.status = status
    self.headers = Dictionary(
      headers.map { ($0.key.lowercased(), $0.value) }, uniquingKeysWith: { _, new in new })
    self.body = body
  }
}

public protocol HTTPTransport: Sendable {
  func send(_ request: URLRequest) async throws -> HTTPResponse
}

final class RefuseRedirects: NSObject, URLSessionTaskDelegate {
  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
    completionHandler: @escaping @Sendable (URLRequest?) -> Void
  ) {
    completionHandler(nil)
  }
}

public final class URLSessionTransport: HTTPTransport, Sendable {
  private let session: URLSession
  public init(configuration: URLSessionConfiguration = .ephemeral) {
    configuration.httpCookieStorage = nil
    configuration.httpShouldSetCookies = false
    configuration.urlCache = nil
    configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
    configuration.timeoutIntervalForRequest = 30
    session = URLSession(
      configuration: configuration, delegate: RefuseRedirects(), delegateQueue: nil)
  }
  public func send(_ request: URLRequest) async throws -> HTTPResponse {
    let (data, response) = try await session.data(for: request)
    guard let response = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
    return HTTPResponse(
      status: response.statusCode,
      headers: Dictionary(
        response.allHeaderFields.map { (String(describing: $0.key), String(describing: $0.value)) },
        uniquingKeysWith: { _, new in new }), body: data)
  }
}

public struct APIError: Error, LocalizedError, Sendable {
  public let status: Int
  public let code: String?
  public let message: String
  public let requestId: String?
  public let retryAfter: TimeInterval?
  public var errorDescription: String? { message }

  public init(response: HTTPResponse, now: Date = Date()) {
    struct Body: Decodable {
      let code: String?
      let message: String?
    }
    let body = try? JSONDecoder().decode(Body.self, from: response.body)
    status = response.status
    code = body?.code
    message =
      body?.message
      ?? (response.status == 404
        ? "This content is unavailable." : "The server could not complete the request. Try again.")
    requestId = response.headers["x-request-id"]
    retryAfter = Self.retryDelay(response.headers["retry-after"], now: now)
  }

  public static func retryDelay(_ value: String?, now: Date) -> TimeInterval? {
    guard let value else { return nil }
    if let seconds = Double(value), seconds.isFinite { return max(0, seconds) }
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.timeZone = TimeZone(secondsFromGMT: 0)
    formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
    return formatter.date(from: value).map { max(0, $0.timeIntervalSince(now)) }
  }
}

public actor APIClient {
  public nonisolated let origin: TrustedOrigin
  private let transport: any HTTPTransport
  private let credentials: any CredentialStore
  public nonisolated let imageCache: (any ImageCaching)?
  private struct AuthorizedRead: Hashable {
    let path: String
    let accountId: UUID?
    let token: String?
  }
  private let images = SharedRequests<AuthorizedRead, Data>()
  private let profiles = SharedRequests<AuthorizedRead, PublicProfile>()
  private var profileAliases: [String: UUID] = [:]
  private var activeImages = 0
  private var waitingImages: [CheckedContinuation<Void, Never>] = []
  public init(
    origin: TrustedOrigin, transport: any HTTPTransport, credentials: any CredentialStore,
    imageCache: (any ImageCaching)? = nil
  ) {
    self.origin = origin
    self.transport = transport
    self.credentials = credentials
    self.imageCache = imageCache
  }

  public func get<T: Decodable & Sendable>(
    path: String, query: [URLQueryItem] = [], accountId: UUID? = nil
  ) async throws -> T {
    try await send(method: "GET", path: path, query: query, accountId: accountId)
  }

  public func send<T: Decodable & Sendable>(
    method: String, path: String, query: [URLQueryItem] = [], body: Data? = nil,
    accountId: UUID? = nil, token: String? = nil, headers: [String: String] = [:]
  ) async throws -> T {
    let response = try await raw(
      method: method, path: path, query: query, body: body, accountId: accountId, token: token,
      headers: headers)
    return try APIJSON.decoder().decode(T.self, from: response.body)
  }

  public func raw(
    method: String, path: String, query: [URLQueryItem] = [], body: Data? = nil,
    accountId: UUID? = nil, token: String? = nil, headers: [String: String] = [:],
    acceptingNotModified: Bool = false
  ) async throws -> HTTPResponse {
    let url = try origin.resolve(path)
    var components = URLComponents(url: url, resolvingAgainstBaseURL: false)!
    if !query.isEmpty { components.queryItems = query }
    var request = URLRequest(url: components.url!, cachePolicy: .reloadIgnoringLocalCacheData)
    request.httpMethod = method
    request.httpBody = body
    request.httpShouldHandleCookies = false
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    if body != nil { request.setValue("application/json", forHTTPHeaderField: "Content-Type") }
    for (name, value) in headers { request.setValue(value, forHTTPHeaderField: name) }
    var bearer = token
    if let accountId, bearer == nil {
      guard let credential = try await credentials.read(accountId: accountId),
        !credential.revocationOnly
      else { throw AuthenticationError.signInRequired }
      bearer = credential.token
    }
    if let bearer { request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }
    let response = try await transport.send(request)
    guard (200..<300).contains(response.status) || (acceptingNotModified && response.status == 304)
    else { throw APIError(response: response) }
    return response
  }

  public func capabilities() async throws -> ClientCapabilities? {
    do { return try await get(path: "/api/client-capabilities") } catch let error as APIError
      where error.status == 404
    { return nil }
  }

  public func voyages(before: String? = nil, owner: UUID? = nil, viewer: UUID? = nil) async throws
    -> VoyagePage
  {
    var query = [URLQueryItem]()
    if let before { query.append(URLQueryItem(name: "before", value: before)) }
    if owner != nil { query.append(URLQueryItem(name: "mine", value: "true")) }
    return try await get(path: "/api/voyages", query: query, accountId: owner ?? viewer)
  }
  public func voyage(_ id: UUID, accountId: UUID? = nil) async throws -> RemoteVoyage {
    try await get(path: "/api/voyages/\(id)", accountId: accountId)
  }
  public func me(_ accountId: UUID) async throws -> AccountResponse {
    try await get(path: "/api/me", accountId: accountId)
  }
  public func profile(_ identifier: String, before: String? = nil, accountId: UUID? = nil)
    async throws -> PublicProfile
  {
    let name: String
    if let id = UUID(uuidString: identifier) {
      name = id.uuidString.lowercased()
    } else if let normalized = try? DisplayName.normalize(identifier) {
      name = normalized
    } else {
      throw OriginError.invalid
    }
    let canonical = profileAliases[name]?.uuidString.lowercased() ?? name
    let token: String?
    if let accountId {
      guard let credential = try await credentials.read(accountId: accountId),
        !credential.revocationOnly
      else { throw AuthenticationError.signInRequired }
      token = credential.token
    } else {
      token = nil
    }
    let path = "/api/profiles/\(canonical)"
    let query = before.map { [URLQueryItem(name: "before", value: $0)] } ?? []
    let key = AuthorizedRead(path: "\(path)|\(before ?? "")", accountId: accountId, token: token)
    let profile = try await profiles.value(for: key) {
      try await self.send(method: "GET", path: path, query: query, token: token)
    }
    if let expected = UUID(uuidString: canonical), expected != profile.user.id {
      throw WorkspaceError.malformedReceipt
    }
    profileAliases[name] = profile.user.id
    profileAliases[profile.user.id.uuidString.lowercased()] = profile.user.id
    if let current = profile.user.displayName { profileAliases[current] = profile.user.id }
    return profile
  }
  public func image(_ path: String, accountId: UUID? = nil) async throws -> Data {
    let url = try origin.resolve(path)
    let parts = url.path.split(separator: "/")
    guard parts.count == 4, parts[0] == "api", parts[1] == "photos",
      UUID(uuidString: String(parts[2])) != nil,
      ["thumb", "display", "full"].contains(String(parts[3]))
    else { throw OriginError.untrusted }
    let token: String?
    if let accountId {
      guard let credential = try await credentials.read(accountId: accountId),
        !credential.revocationOnly
      else {
        throw AuthenticationError.signInRequired
      }
      token = credential.token
    } else {
      token = nil
    }
    let request = AuthorizedRead(path: path, accountId: accountId, token: token)
    return try await images.value(for: request) { try await self.downloadImage(request) }
  }

  private func downloadImage(_ request: AuthorizedRead) async throws -> Data {
    if activeImages < 4 {
      activeImages += 1
    } else {
      await withCheckedContinuation { waitingImages.append($0) }
    }
    defer {
      if waitingImages.isEmpty { activeImages -= 1 } else { waitingImages.removeFirst().resume() }
    }
    let key =
      "\(origin.url.absoluteString)|\(request.accountId?.uuidString ?? "anonymous")|\(request.path)"
    let cached = try? await imageCache?.lookup(key, now: Date())
    do {
      let response = try await raw(
        method: "GET", path: request.path, token: request.token,
        headers: cached.map { ["If-None-Match": $0.etag] } ?? [:], acceptingNotModified: true)
      if response.status == 304 {
        guard let cached else { throw URLError(.badServerResponse) }
        return cached.data
      }
      try? await imageCache?.store(
        response.body, etag: response.headers["etag"], key: key, now: Date())
      return response.body
    } catch {
      // A denied or unavailable request never falls back to an ordinary cached image.
      try? await imageCache?.store(Data(), etag: nil, key: key, now: Date())
      throw error
    }
  }
}
