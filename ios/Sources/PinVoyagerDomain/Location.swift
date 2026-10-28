import Foundation

public struct Coordinate: Codable, Equatable, Hashable, Sendable {
  public let latitude: Double
  public let longitude: Double
  public let precisionM: Double?
  public init(latitude: Double, longitude: Double, precisionM: Double? = nil) {
    self.latitude = latitude
    self.longitude = longitude
    self.precisionM = precisionM
  }
  public var isValid: Bool {
    latitude.isFinite && longitude.isFinite && (-90...90).contains(latitude)
      && (-180...180).contains(longitude)
  }
}

public enum Granularity: String, CaseIterable, Codable, Sendable {
  case exact, km1, km10, km50, unsupported
  public static var allCases: [Self] { [.exact, .km1, .km10, .km50] }
  public init(from decoder: any Decoder) throws {
    self = Self(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unsupported
  }
  public var canPublish: Bool { self != .unsupported }
  public var label: String {
    switch self {
    case .exact: "Exact"
    case .km1: "Approximately 1 km"
    case .km10: "Approximately 10 km"
    case .km50: "Approximately 50 km"
    case .unsupported: "Unknown precision — update required"
    }
  }
  public var halfCellM: Double? {
    switch self {
    case .exact: nil
    case .km1: 500
    case .km10: 5_000
    case .km50, .unsupported: 25_000
    }
  }
  /// Mirrors `voyages::granularity::Granularity::apply`, including floor division below zero.
  public func apply(_ coordinate: Coordinate) -> Coordinate {
    guard let halfCellM else { return coordinate }
    let cell = halfCellM * 2 / 1_000 / 111.32
    func center(_ value: Double, _ width: Double) -> Double {
      floor(value / width) * width + width / 2
    }
    let latitude = min(90, max(-90, center(coordinate.latitude, cell)))
    let longitudeCell = cell / cos(min(abs(latitude), 85) * .pi / 180)
    let longitude = MapGeometry.wrapLongitude(center(coordinate.longitude, longitudeCell))
    return Coordinate(latitude: latitude, longitude: longitude, precisionM: halfCellM)
  }
}

public struct MapBounds: Equatable, Sendable {
  public let latitude: Double
  public let longitude: Double
  public let latitudeSpan: Double
  public let longitudeSpan: Double
}

public enum MapGeometry {
  public static func wrapLongitude(_ value: Double) -> Double {
    let remainder = (value + 180).truncatingRemainder(dividingBy: 360)
    return (remainder < 0 ? remainder + 360 : remainder) - 180
  }

  public static func discoveryZoom(width: Double, worldFraction: Double) -> Int {
    guard width > 0, worldFraction > 0, width.isFinite, worldFraction.isFinite else { return 0 }
    return Int(min(20, max(0, log2(width / (512 * worldFraction)).rounded())))
  }

  public static func bounds(_ coordinates: [Coordinate]) -> MapBounds? {
    let valid = coordinates.filter(\.isValid)
    guard let first = valid.first else { return nil }
    let longitudes = valid.map { wrapLongitude($0.longitude) }.sorted()
    var gap = -Double.infinity
    var start = first.longitude
    for index in longitudes.indices {
      let next = index + 1 < longitudes.count ? longitudes[index + 1] : longitudes[0] + 360
      if next - longitudes[index] > gap {
        gap = next - longitudes[index]
        start = next
      }
    }
    let span = max(0, 360 - gap)
    let low = valid.map(\.latitude).min()!
    let high = valid.map(\.latitude).max()!
    return MapBounds(
      latitude: (low + high) / 2, longitude: wrapLongitude(start + span / 2),
      latitudeSpan: high - low, longitudeSpan: span)
  }
}

public struct MapPhotoPin: Identifiable, Equatable, Sendable {
  public let id: UUID
  public let coordinate: Coordinate
  public init(id: UUID, coordinate: Coordinate) {
    self.id = id
    self.coordinate = coordinate
  }
}

public struct PhotoPinCluster: Identifiable, Equatable, Sendable {
  public let id: UUID
  public let coordinate: Coordinate
  public let members: [UUID]
}

extension MapGeometry {
  /// Uses the same seed, 44-point grid, and wrapped distance as `web/src/map/geo.ts::clusterPins`.
  public static func cluster(_ pins: [MapPhotoPin], zoom: Double, radius: Double = 44)
    -> [PhotoPinCluster]
  {
    guard radius > 0, radius.isFinite, zoom.isFinite else { return [] }
    let pins = pins.filter { $0.coordinate.isValid }
    let size = 512 * pow(2, min(22, max(0, zoom)))
    struct Cell: Hashable {
      let x: Int
      let y: Int
    }
    func cell(_ x: Double, _ y: Double) -> Cell {
      Cell(x: Int(floor(x / radius)), y: Int(floor(y / radius)))
    }
    let points = pins.map { pin -> (x: Double, y: Double) in
      let latitude = min(85.051129, max(-85.051129, pin.coordinate.latitude))
      let sine = sin(latitude * .pi / 180)
      return (
        (wrapLongitude(pin.coordinate.longitude) + 180) / 360 * size,
        (0.5 - log((1 + sine) / (1 - sine)) / (4 * .pi)) * size
      )
    }
    var cells: [Cell: Set<Int>] = [:]
    let keys = points.map { point in [-size, 0, size].map { cell(point.x + $0, point.y) } }
    for index in keys.indices { for key in keys[index] { cells[key, default: []].insert(index) } }
    var taken: Set<Int> = []
    var clusters: [PhotoPinCluster] = []
    for index in points.indices where !taken.contains(index) {
      let seed = points[index]
      var candidates: Set<Int> = []
      for dx in [-radius, 0, radius] {
        for dy in [-radius, 0, radius] {
          candidates.formUnion(cells[cell(seed.x + dx, seed.y + dy)] ?? [])
        }
      }
      var members: [UUID] = []
      var sumX = 0.0
      var sumY = 0.0
      for candidate in candidates.sorted() {
        let point = points[candidate]
        let raw = point.x - seed.x
        let dx = raw - (raw / size).rounded() * size
        let dy = point.y - seed.y
        guard dx * dx + dy * dy <= radius * radius else { continue }
        taken.insert(candidate)
        for key in keys[candidate] { cells[key]?.remove(candidate) }
        members.append(pins[candidate].id)
        sumX += seed.x + dx
        sumY += point.y
      }
      guard !members.isEmpty else { continue }
      let count = Double(members.count)
      let coordinate =
        members.count == 1
        ? pins[index].coordinate
        : Coordinate(
          latitude: atan(sinh(.pi - 2 * .pi * sumY / count / size)) * 180 / .pi,
          longitude: wrapLongitude(sumX / count / size * 360 - 180))
      clusters.append(PhotoPinCluster(id: pins[index].id, coordinate: coordinate, members: members))
    }
    return clusters
  }
}
