import Foundation
import PinVoyagerDomain

public enum APIJSON {
  public static func date(_ value: String) -> Date? {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let date = formatter.date(from: value) { return date }
    formatter.formatOptions = [.withInternetDateTime]
    return formatter.date(from: value)
  }

  public static func decoder() -> JSONDecoder {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
  }
  public static func encode<T: Encodable>(_ value: T) throws -> Data {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.dateEncodingStrategy = .iso8601
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    return try encoder.encode(value)
  }
}

public struct VoyageSummary: Codable, Identifiable, Sendable {
  public let id: UUID
  public let ownerId: UUID
  public let title: String
  public let description: String?
  public let published: Bool
  public let granularity: Granularity
  public let granularityLabel: String
  public let coverUrl: String?
  public let photosPending: Bool
  public let createdAt: String
}

public struct VoyagePage: Codable, Sendable {
  public let voyages: [VoyageSummary]
  public let nextBefore: String?
}

public struct RemotePhoto: Codable, Identifiable, Sendable {
  public let id: UUID
  public let position: Int
  public let capturedAt: String?
  public let width: Int
  public let height: Int
  public let variantStatus: VariantState
  public let thumbnailUrl: String
  public let displayUrl: String
  public let fullUrl: String
  public let coordinate: Coordinate?
  public let placed: Bool
}

public struct OwnerPhoto: Decodable, Sendable {
  public let id: UUID
  public let width: Int
  public let height: Int
  public let capturedAt: String?
  public let latitude: Double?
  public let longitude: Double?
  public let variantStatus: VariantState
  public var coordinate: Coordinate? {
    guard let latitude, let longitude else { return nil }
    let coordinate = Coordinate(latitude: latitude, longitude: longitude)
    return coordinate.isValid ? coordinate : nil
  }
}
public struct OwnerPhotoPage: Decodable, Sendable {
  public let photos: [OwnerPhoto]
  public let nextBefore: String?
}

public struct RemoteVoyage: Codable, Identifiable, Sendable {
  public let id: UUID
  public let ownerId: UUID
  public let title: String
  public let description: String?
  public let published: Bool
  public let granularity: Granularity
  public let granularityLabel: String
  public let photoCount: Int
  public let placedCount: Int
  public let placeCount: Int
  public let unplacedCount: Int
  public let photos: [RemotePhoto]
  public let revision: String?
  public var fields: VoyageFields {
    VoyageFields(title: title, description: description, granularity: granularity)
  }
}

public struct LinkedIdentity: Codable, Identifiable, Sendable {
  public var id: String { provider }
  public let provider: String
  public let linkedAt: String
  public let lastSeenAt: String
}
public struct AccountResponse: Codable, Sendable {
  public let user: UserProfile
  public let identities: [LinkedIdentity]
}
public struct PublicProfile: Codable, Sendable {
  public let user: UserProfile
  public let voyages: [VoyageSummary]
  public let nextBefore: String?
}

public struct ClusterVoyage: Codable, Identifiable, Sendable {
  public let id: UUID
  public let title: String
}
public struct DiscoveryCluster: Codable, Identifiable, Sendable {
  public var id: UUID { photoId }
  public let photoId: UUID
  public let voyageId: UUID
  public let coordinate: Coordinate
  public let count: Int
  public let thumbnailUrl: String
  public let voyages: [ClusterVoyage]
}
public struct DiscoveryResponse: Codable, Sendable {
  public let clusters: [DiscoveryCluster]
  public let truncated: Bool
}

public struct ClientCapabilities: Codable, Sendable {
  public let nativeAuthentication: Bool?
  public let nativeProtocolVersions: [Int]
  public let providers: [String]
  public let formats: [String]
  public let maxUploadBytes: Int64
  public let maxOrderEntries: Int
  public let maxManifestEntries: Int
  public let ownerOfflineDownloads: Bool
  public var supportsNativeSync: Bool { nativeProtocolVersions.contains(1) }
  public var supportsNativeAuthentication: Bool { nativeAuthentication ?? supportsNativeSync }
}
