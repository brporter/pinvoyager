import Foundation
import PinVoyagerDomain

public protocol DevicePreferences: Sendable {
  func load() async throws -> SyncPolicy
  func save(_ policy: SyncPolicy) async throws
}

/// Device settings contain no account records, source paths, or credentials.
public actor UserDefaultsDevicePreferences: DevicePreferences {
  public static let policyKey = "native.sync-policy"
  private let suiteName: String?
  public init(suiteName: String? = nil) { self.suiteName = suiteName }
  private func defaults() throws -> UserDefaults {
    guard let suiteName else { return .standard }
    guard let defaults = UserDefaults(suiteName: suiteName) else {
      throw CocoaError(.fileReadNoPermission)
    }
    return defaults
  }
  public func load() throws -> SyncPolicy {
    guard let data = try defaults().data(forKey: Self.policyKey) else { return SyncPolicy() }
    return try JSONDecoder().decode(SyncPolicy.self, from: data)
  }
  public func save(_ policy: SyncPolicy) throws {
    try defaults().set(JSONEncoder().encode(policy), forKey: Self.policyKey)
  }
}
