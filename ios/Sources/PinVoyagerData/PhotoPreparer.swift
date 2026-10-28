import CoreGraphics
import Foundation
import ImageIO
import PinVoyagerDomain
import UniformTypeIdentifiers

public actor PhotoPreparer {
  private let vault: FileVault
  public init(vault: FileVault) { self.vault = vault }

  public func prepare(
    _ input: URL, accountId: UUID, voyageId: UUID, maxBytes: Int64, photoId: UUID = UUID(),
    uploadKey: UUID = UUID()
  ) async throws -> LocalPhoto {
    try await prepare(
      input, accountId: accountId, voyageId: voyageId, maxBytes: maxBytes, photoId: photoId,
      uploadKey: uploadKey, source: nil)
  }

  public func recover(_ pending: LocalPhoto, accountId: UUID, maxBytes: Int64) async throws
    -> LocalPhoto
  {
    let source = try await vault.inspectOriginal(
      accountId: accountId, photoId: pending.id, maxBytes: maxBytes)
    let input = try await vault.url(source.relativePath, accountId: accountId)
    var photo = try await prepare(
      input, accountId: accountId, voyageId: pending.voyageId, maxBytes: maxBytes,
      photoId: pending.id, uploadKey: pending.uploadKey, source: source)
    photo.filename = pending.filename
    return photo
  }

  public func smallerCopy(_ input: URL, maxDimension: Int = 2048) throws -> URL {
    guard (32...4096).contains(maxDimension) else {
      throw ValidationError(
        field: "photo", message: "Choose a JPEG size between 32 and 4096 pixels.")
    }
    guard input.isFileURL,
      let source = CGImageSourceCreateWithURL(
        input as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
      let image = CGImageSourceCreateThumbnailAtIndex(
        source, 0,
        [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceCreateThumbnailWithTransform: true,
          kCGImageSourceThumbnailMaxPixelSize: maxDimension,
        ] as CFDictionary)
    else { throw FileVaultError.unsupportedFormat }
    let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any] ?? [:]
    var metadata: [CFString: Any] = [
      kCGImageDestinationLossyCompressionQuality: 0.8, kCGImagePropertyOrientation: 1,
    ]
    metadata[kCGImagePropertyGPSDictionary] = properties[kCGImagePropertyGPSDictionary]
    if let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any],
      let captured = exif[kCGImagePropertyExifDateTimeOriginal]
    {
      metadata[kCGImagePropertyExifDictionary] = [kCGImagePropertyExifDateTimeOriginal: captured]
    }
    let url = URL.temporaryDirectory.appendingPathComponent("Smaller-\(UUID()).jpg")
    guard
      let destination = CGImageDestinationCreateWithURL(
        url as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
    else { throw FileVaultError.unsupportedFormat }
    CGImageDestinationAddImage(destination, image, metadata as CFDictionary)
    guard CGImageDestinationFinalize(destination) else {
      try? FileManager.default.removeItem(at: url)
      throw FileVaultError.unsupportedFormat
    }
    return url
  }

  private func prepare(
    _ input: URL, accountId: UUID, voyageId: UUID, maxBytes: Int64, photoId: UUID, uploadKey: UUID,
    source: SourceFile?
  ) async throws -> LocalPhoto {
    guard
      let image = CGImageSourceCreateWithURL(
        input as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
      let type = CGImageSourceGetType(image),
      [
        UTType.jpeg.identifier, UTType.png.identifier, UTType.webP.identifier,
        UTType.heic.identifier, UTType.heif.identifier,
      ].contains(type as String),
      let properties = CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any],
      let width = properties[kCGImagePropertyPixelWidth] as? Int,
      let height = properties[kCGImagePropertyPixelHeight] as? Int,
      width > 0, height > 0, Int64(width) * Int64(height) <= 100_000_000
    else { throw FileVaultError.unsupportedFormat }
    var photo = LocalPhoto(
      id: photoId, voyageId: voyageId, uploadKey: uploadKey, filename: input.lastPathComponent)
    photo.width = width
    photo.height = height
    if let gps = properties[kCGImagePropertyGPSDictionary] as? [CFString: Any],
      let latitude = gps[kCGImagePropertyGPSLatitude] as? Double,
      let longitude = gps[kCGImagePropertyGPSLongitude] as? Double
    {
      let coordinate = Coordinate(
        latitude: (gps[kCGImagePropertyGPSLatitudeRef] as? String) == "S" ? -latitude : latitude,
        longitude: (gps[kCGImagePropertyGPSLongitudeRef] as? String) == "W" ? -longitude : longitude
      )
      if coordinate.isValid { photo.coordinate = coordinate }
    }
    if let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any] {
      photo.capturedAt = exif[kCGImagePropertyExifDateTimeOriginal] as? String
    }
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 512,
      kCGImageSourceShouldCacheImmediately: true,
    ]
    guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(image, 0, options as CFDictionary)
    else { throw FileVaultError.unsupportedFormat }
    let preview = NSMutableData()
    guard
      let destination = CGImageDestinationCreateWithData(
        preview, UTType.jpeg.identifier as CFString, 1, nil)
    else { throw FileVaultError.unsupportedFormat }
    CGImageDestinationAddImage(
      destination, thumbnail, [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else { throw FileVaultError.unsupportedFormat }
    if let source {
      photo.source = source
    } else {
      photo.source = try await vault.importFile(
        input, accountId: accountId, photoId: photo.id, maxBytes: maxBytes)
    }
    let previewPath = "previews/\(photo.id).jpg"
    try await vault.write(preview as Data, relativePath: previewPath, accountId: accountId)
    photo.previewPath = previewPath
    photo.transfer = .saved
    return photo
  }
}
