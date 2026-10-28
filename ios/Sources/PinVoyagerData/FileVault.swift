import CryptoKit
import Foundation
import PinVoyagerDomain

public enum FileVaultError: Error, Equatable, LocalizedError, Sendable {
  case invalidPath, unsupportedFormat, tooLarge, insufficientSpace, missingSource
  public var errorDescription: String? {
    switch self {
    case .invalidPath: "The file path is outside this account's workspace."
    case .unsupportedFormat: "Choose a HEIC, JPEG, PNG or WebP still image."
    case .tooLarge: "This photo exceeds the server upload limit. Keep it or choose a smaller copy."
    case .insufficientSpace:
      "There is not enough space to save and prepare this photo. Free space and try again."
    case .missingSource: "Choose this photo again to recover its missing file."
    }
  }
}

public actor FileVault {
  public let root: URL
  public init(root: URL) throws {
    self.root = root.standardizedFileURL
    try Self.protectDirectory(root)
  }

  public func url(_ relativePath: String, accountId: UUID) throws -> URL {
    guard !relativePath.hasPrefix("/"), !relativePath.split(separator: "/").contains(".."),
      !relativePath.isEmpty
    else { throw FileVaultError.invalidPath }
    let directory = root.appendingPathComponent(accountId.uuidString, isDirectory: true)
    let candidate = directory.appendingPathComponent(relativePath).standardizedFileURL
    guard
      candidate.resolvingSymlinksInPath().path.hasPrefix(
        directory.resolvingSymlinksInPath().path + "/")
    else { throw FileVaultError.invalidPath }
    return candidate
  }

  public func read(_ relativePath: String, accountId: UUID) throws -> Data {
    try Data(contentsOf: url(relativePath, accountId: accountId))
  }

  public func importFile(_ source: URL, accountId: UUID, photoId: UUID, maxBytes: Int64) throws
    -> SourceFile
  {
    guard source.isFileURL, FileManager.default.fileExists(atPath: source.path) else {
      throw FileVaultError.missingSource
    }
    let values = try source.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
    guard values.isRegularFile == true else { throw FileVaultError.missingSource }
    let size = Int64(values.fileSize ?? 0)
    guard size > 0, size <= maxBytes else { throw FileVaultError.tooLarge }
    let available =
      try root.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
      .volumeAvailableCapacityForImportantUsage ?? Int64.max
    guard size <= (available - 100 * 1_024 * 1_024) / 2 else {
      throw FileVaultError.insufficientSpace
    }
    let relative = "originals/\(photoId.uuidString)"
    let destination = try url(relative, accountId: accountId)
    try Self.protectDirectory(destination.deletingLastPathComponent())
    let staging = destination.appendingPathExtension("part")
    let input = try FileHandle(forReadingFrom: source)
    defer { try? input.close() }
    let header = try input.read(upToCount: 32) ?? Data()
    let mediaType = try Self.mediaType(header)
    try input.seek(toOffset: 0)
    FileManager.default.createFile(atPath: staging.path, contents: nil)
    let output = try FileHandle(forWritingTo: staging)
    var hash = SHA256()
    var count: Int64 = 0
    do {
      while let chunk = try input.read(upToCount: 1_024 * 1_024), !chunk.isEmpty {
        count += Int64(chunk.count)
        guard count <= maxBytes else { throw FileVaultError.tooLarge }
        hash.update(data: chunk)
        try output.write(contentsOf: chunk)
      }
      try output.synchronize()
      try output.close()
      guard !FileManager.default.fileExists(atPath: destination.path) else {
        throw CocoaError(.fileWriteFileExists)
      }
      try FileManager.default.moveItem(at: staging, to: destination)
      try Self.protectFile(destination)
    } catch {
      try? output.close()
      try? FileManager.default.removeItem(at: staging)
      throw error
    }
    return SourceFile(
      relativePath: relative,
      contentHash: hash.finalize().map { String(format: "%02x", $0) }.joined(), byteCount: count,
      mediaType: mediaType)
  }

  public func inspectOriginal(accountId: UUID, photoId: UUID, maxBytes: Int64) throws -> SourceFile
  {
    let path = "originals/\(photoId)"
    let source = try url(path, accountId: accountId)
    guard FileManager.default.fileExists(atPath: source.path),
      try source.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true
    else { throw FileVaultError.missingSource }
    let file = try FileHandle(forReadingFrom: source)
    defer { try? file.close() }
    let type = try Self.mediaType(try file.read(upToCount: 32) ?? Data())
    try file.seek(toOffset: 0)
    var hash = SHA256()
    var count: Int64 = 0
    while let chunk = try file.read(upToCount: 1_024 * 1_024), !chunk.isEmpty {
      count += Int64(chunk.count)
      guard count <= maxBytes else { throw FileVaultError.tooLarge }
      hash.update(data: chunk)
    }
    guard count > 0 else { throw FileVaultError.missingSource }
    return SourceFile(
      relativePath: path, contentHash: hash.finalize().map { String(format: "%02x", $0) }.joined(),
      byteCount: count, mediaType: type)
  }

  public func write(_ data: Data, relativePath: String, accountId: UUID) throws {
    let destination = try url(relativePath, accountId: accountId)
    try Self.protectDirectory(destination.deletingLastPathComponent())
    try data.write(to: destination, options: .atomic)
    try Self.protectFile(destination)
  }

  public func remove(_ relativePath: String, accountId: UUID) throws {
    let file = try url(relativePath, accountId: accountId)
    if FileManager.default.fileExists(atPath: file.path) {
      try FileManager.default.removeItem(at: file)
    }
  }

  public func removeAccount(_ accountId: UUID) throws {
    let directory = root.appendingPathComponent(accountId.uuidString)
    if FileManager.default.fileExists(atPath: directory.path) {
      try FileManager.default.removeItem(at: directory)
    }
  }

  public struct ExportPreview: Codable, Equatable, Sendable {
    public let includedFiles: Int
    public let byteCount: Int64
    public let missingOriginals: [String]
  }
  public func exportPreview(_ workspace: Workspace, voyageId: UUID) throws -> ExportPreview {
    guard workspace.account.access != .locked else { throw WorkspaceError.locked }
    guard workspace.voyages.contains(where: { $0.id == voyageId }) else {
      throw WorkspaceError.missingVoyage
    }
    var included = 0
    var bytes: Int64 = 0
    var missing: [String] = []
    for photo in workspace.photos.filter({ $0.voyageId == voyageId }).sorted(by: {
      $0.order < $1.order
    }) {
      guard let source = photo.source,
        FileManager.default.fileExists(
          atPath: try url(source.relativePath, accountId: workspace.account.id).path)
      else {
        missing.append(photo.filename)
        continue
      }
      included += 1
      bytes += source.byteCount
    }
    return ExportPreview(includedFiles: included, byteCount: bytes, missingOriginals: missing)
  }

  public func export(_ workspace: Workspace, voyageId: UUID? = nil, destination: URL) throws -> URL
  {
    guard workspace.account.access != .locked else { throw WorkspaceError.locked }
    var workspace = workspace
    if let voyageId {
      let preview = try exportPreview(workspace, voyageId: voyageId)
      try Self.protectDirectory(destination)
      try JSONEncoder().encode(preview).write(
        to: destination.appendingPathComponent("export-preview.json"), options: .atomic)
      workspace.voyages = workspace.voyages.filter { $0.id == voyageId }
      workspace.photos = workspace.photos.filter { $0.voyageId == voyageId }
      workspace.operations = workspace.operations.filter { $0.voyageId == voyageId }
      workspace.workingCopies = workspace.workingCopies.filter { $0.voyageId == voyageId }
      workspace.publicationIntents = workspace.publicationIntents.filter { $0.voyageId == voyageId }
      let operations = Set(workspace.operations.map(\.id))
      workspace.transfers = workspace.transfers.filter { operations.contains($0.operationId) }
      workspace.blockedAccounts = []
      workspace.scratch = nil
    }
    try Self.protectDirectory(destination)
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    try encoder.encode(workspace).write(
      to: destination.appendingPathComponent("workspace.json"), options: .atomic)
    for photo in workspace.photos {
      guard let source = photo.source else { continue }
      let original = try url(source.relativePath, accountId: workspace.account.id)
      if FileManager.default.fileExists(atPath: original.path) {
        try FileManager.default.copyItem(
          at: original, to: destination.appendingPathComponent(photo.id.uuidString))
      }
    }
    return destination
  }

  public func exportArchive(_ workspace: Workspace, voyageId: UUID, destination: URL) throws -> URL
  {
    let directory = try export(workspace, voyageId: voyageId, destination: destination)
    let archive = destination.appendingPathExtension("zip")
    var coordinationError: NSError?
    var copyError: (any Error)?
    NSFileCoordinator().coordinate(
      readingItemAt: directory, options: .forUploading, error: &coordinationError
    ) { snapshot in
      do { try FileManager.default.copyItem(at: snapshot, to: archive) } catch { copyError = error }
    }
    if let coordinationError { throw coordinationError }
    if let copyError { throw copyError }
    try Self.protectFile(archive)
    return archive
  }

  static func protectDirectory(_ url: URL) throws {
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    try protectFile(url)
  }

  static func protectFile(_ url: URL) throws {
    var url = url
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try url.setResourceValues(values)
    #if os(iOS)
      try FileManager.default.setAttributes(
        [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
        ofItemAtPath: url.path)
    #endif
  }

  static func mediaType(_ header: Data) throws -> String {
    let bytes = [UInt8](header)
    if bytes.starts(with: [0xff, 0xd8, 0xff]) { return "image/jpeg" }
    if bytes.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]) { return "image/png" }
    if bytes.count >= 12, String(decoding: bytes[0..<4], as: UTF8.self) == "RIFF",
      String(decoding: bytes[8..<12], as: UTF8.self) == "WEBP"
    {
      return "image/webp"
    }
    if bytes.count >= 12, String(decoding: bytes[4..<8], as: UTF8.self) == "ftyp",
      ["heic", "heix", "hevc", "hevx", "mif1"].contains(
        String(decoding: bytes[8..<12], as: UTF8.self))
    {
      return "image/heic"
    }
    throw FileVaultError.unsupportedFormat
  }
}

public struct MultipartBody: Sendable {
  public let url: URL
  public let byteCount: Int64
  public let contentType: String

  public static func write(
    source: URL, destination: URL, mediaType: String, boundary: String, maxBytes: Int64
  ) throws -> Self {
    guard ["image/jpeg", "image/png", "image/webp", "image/heic"].contains(mediaType),
      boundary.range(of: "^[A-Za-z0-9-]{1,70}$", options: .regularExpression) != nil
    else { throw FileVaultError.unsupportedFormat }
    let prefix = Data(
      "--\(boundary)\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo\"\r\nContent-Type: \(mediaType)\r\n\r\n"
        .utf8)
    let suffix = Data("\r\n--\(boundary)--\r\n".utf8)
    let size =
      Int64(try source.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
      + Int64(prefix.count + suffix.count)
    guard size <= maxBytes else { throw FileVaultError.tooLarge }
    try FileVault.protectDirectory(destination.deletingLastPathComponent())
    let staging = destination.appendingPathExtension("part")
    FileManager.default.createFile(atPath: staging.path, contents: nil)
    let input = try FileHandle(forReadingFrom: source)
    let output = try FileHandle(forWritingTo: staging)
    defer {
      try? input.close()
      try? output.close()
    }
    do {
      try output.write(contentsOf: prefix)
      while let bytes = try input.read(upToCount: 1_024 * 1_024), !bytes.isEmpty {
        try output.write(contentsOf: bytes)
      }
      try output.write(contentsOf: suffix)
      try output.synchronize()
      try output.close()
      try FileManager.default.moveItem(at: staging, to: destination)
      try FileVault.protectFile(destination)
      return Self(
        url: destination, byteCount: size, contentType: "multipart/form-data; boundary=\(boundary)")
    } catch {
      try? FileManager.default.removeItem(at: staging)
      throw error
    }
  }
}
