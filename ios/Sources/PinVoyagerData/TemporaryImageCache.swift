import CryptoKit
import Foundation

public struct CachedImage: Codable, Equatable, Sendable {
  public let data: Data
  public let etag: String
}

public protocol ImageCaching: Sendable {
  func lookup(_ key: String, now: Date) async throws -> CachedImage?
  func store(_ data: Data, etag: String?, key: String, now: Date) async throws
  func byteCount() async throws -> Int64
  func clear() async throws
}

/// Ordinary variants remain unusable until a fresh server response authorizes reuse.
public actor TemporaryImageCache: ImageCaching {
  private let directory: URL
  private let capacity: Int64
  public init(directory: URL, capacity: Int64 = 250 * 1_024 * 1_024) throws {
    self.directory = directory
    self.capacity = max(0, capacity)
    try FileVault.protectDirectory(directory)
  }
  private func path(_ key: String) -> URL {
    let hash = SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined()
    return directory.appendingPathComponent(hash).appendingPathExtension("cache")
  }
  public func lookup(_ key: String, now: Date = Date()) throws -> CachedImage? {
    let url = path(key)
    guard FileManager.default.fileExists(atPath: url.path) else { return nil }
    guard let entry = try? JSONDecoder().decode(CachedImage.self, from: Data(contentsOf: url))
    else {
      try FileManager.default.removeItem(at: url)
      return nil
    }
    try FileManager.default.setAttributes([.modificationDate: now], ofItemAtPath: url.path)
    return entry
  }
  public func store(_ data: Data, etag: String?, key: String, now: Date = Date()) throws {
    let url = path(key)
    if FileManager.default.fileExists(atPath: url.path) {
      try FileManager.default.removeItem(at: url)
    }
    guard let etag, !etag.isEmpty, data.count <= capacity else { return }
    let encoded = try JSONEncoder().encode(CachedImage(data: data, etag: etag))
    guard encoded.count <= capacity else { return }
    try encoded.write(to: url, options: .atomic)
    try FileVault.protectFile(url)
    try FileManager.default.setAttributes([.modificationDate: now], ofItemAtPath: url.path)
    let files = try entries().sorted { $0.date < $1.date }
    var total = files.reduce(Int64(0)) { $0 + $1.size }
    for file in files where total > capacity {
      try FileManager.default.removeItem(at: file.url)
      total -= file.size
    }
  }
  private func entries() throws -> [(url: URL, size: Int64, date: Date)] {
    try FileManager.default.contentsOfDirectory(
      at: directory,
      includingPropertiesForKeys: [.fileSizeKey, .contentModificationDateKey]
    )
    .filter { $0.pathExtension == "cache" }
    .map { url in
      let values = try url.resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey])
      return (url, Int64(values.fileSize ?? 0), values.contentModificationDate ?? .distantPast)
    }
  }
  public func byteCount() throws -> Int64 { try entries().reduce(0) { $0 + $1.size } }
  public func clear() throws {
    for file in try entries() { try FileManager.default.removeItem(at: file.url) }
  }
}
