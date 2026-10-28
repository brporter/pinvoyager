import Foundation

public enum OriginError: Error, Sendable { case invalid, untrusted }

public struct TrustedOrigin: Equatable, Sendable {
  public let url: URL
  public init(_ url: URL, allowsLocalHTTP: Bool = false) throws {
    let local = ["localhost", "127.0.0.1", "::1"].contains(url.host ?? "")
    guard url.host != nil, url.user == nil, url.password == nil,
      url.query == nil, url.fragment == nil, ["", "/"].contains(url.path),
      url.scheme == "https" || (allowsLocalHTTP && local && url.scheme == "http")
    else { throw OriginError.invalid }
    self.url = url
  }
  public func contains(_ candidate: URL) -> Bool {
    func port(_ url: URL) -> Int { url.port ?? (url.scheme == "https" ? 443 : 80) }
    return candidate.scheme == url.scheme && candidate.host?.lowercased() == url.host?.lowercased()
      && port(candidate) == port(url) && candidate.user == nil && candidate.password == nil
  }
  public func url(path: [String], query: [URLQueryItem] = []) -> URL {
    var components = URLComponents(url: url, resolvingAgainstBaseURL: false)!
    components.path = "/" + path.joined(separator: "/")
    components.queryItems = query.isEmpty ? nil : query
    return components.url!
  }
  public func resolve(_ path: String) throws -> URL {
    guard let candidate = URL(string: path, relativeTo: url)?.absoluteURL, contains(candidate),
      !candidate.path.contains("..")
    else { throw OriginError.untrusted }
    return candidate
  }
}

public enum AppTab: String, CaseIterable, Codable, Sendable {
  case discover, voyages, sync, account
}

public enum AppRoute: Codable, Hashable, Sendable {
  case voyage(UUID)
  case editVoyage(UUID)
  case addPhotos(UUID)
  case profile(String)
  case createVoyage, account

  public static func parse(_ url: URL, origin: TrustedOrigin) -> Self? {
    guard origin.contains(url), url.fragment == nil else { return nil }
    let path = url.path.split(separator: "/").map(String.init)
    let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
    if path.count == 2, path[0] == "voyages", let id = UUID(uuidString: path[1]), items.isEmpty {
      return .voyage(id)
    }
    if path.count == 3, path[0] == "voyages", let id = UUID(uuidString: path[1]), path[2] == "edit",
      items.isEmpty
    {
      return .editVoyage(id)
    }
    if path.count == 2, path[0] == "u", items.isEmpty,
      UUID(uuidString: path[1]) != nil || (try? DisplayName.normalize(path[1])) != nil
    {
      return .profile(path[1])
    }
    if path == ["settings"], items.isEmpty { return .account }
    if path == ["upload"] {
      if items.isEmpty { return .createVoyage }
      if items.count == 1, items[0].name == "voyage", let value = items[0].value,
        let id = UUID(uuidString: value)
      {
        return .addPhotos(id)
      }
    }
    return nil
  }
}
