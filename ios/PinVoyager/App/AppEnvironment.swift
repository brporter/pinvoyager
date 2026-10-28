import Foundation
import PinVoyagerData
import PinVoyagerDomain

struct AppEnvironment: Sendable {
  let testing: Bool
  var directory: URL
  var cacheDirectory: URL
  var apiURL: URL
  var webURL: URL
  var credentials: any CredentialStore
  var deletionRecovery: any DeletionRecoveryStore
  var preferences: any DevicePreferences
  var transport: any HTTPTransport
  var transfers: SystemTransferDriver
  var activeAccountId: UUID?

  @MainActor static func system(testing: Bool) throws -> Self {
    guard
      let apiURL = URL(
        string: Bundle.main.object(forInfoDictionaryKey: "PinVoyagerAPIOrigin") as? String
          ?? "https://pinvoyager.com"),
      let webURL = URL(
        string: Bundle.main.object(forInfoDictionaryKey: "PinVoyagerWebOrigin") as? String
          ?? "https://pinvoyager.com")
    else { throw OriginError.invalid }
    let directory =
      testing
      ? URL.temporaryDirectory.appendingPathComponent(UUID().uuidString) : AppBootstrap.directory
    #if DEBUG
      let transport: any HTTPTransport = testing ? DevelopmentTransport() : URLSessionTransport()
    #else
      let transport: any HTTPTransport = URLSessionTransport()
    #endif
    return Self(
      testing: testing, directory: directory,
      cacheDirectory: testing
        ? directory.appendingPathComponent("cache")
        : URL.cachesDirectory.appendingPathComponent("PinVoyager/images"),
      apiURL: apiURL, webURL: webURL,
      credentials: KeychainCredentialStore(
        service: testing
          ? "com.pinvoyager.ui-test.sessions.\(UUID())" : "com.pinvoyager.native.sessions"),
      deletionRecovery: KeychainDeletionRecoveryStore(
        service: testing
          ? "com.pinvoyager.ui-test.deletions.\(UUID())" : "com.pinvoyager.native.deletions"),
      preferences: UserDefaultsDevicePreferences(
        suiteName: testing
          ? ProcessInfo.processInfo.environment["PINVOYAGER_TEST_PREFERENCES"]
            ?? "com.pinvoyager.ui-test.preferences.\(UUID())" : nil),
      transport: transport, transfers: .shared,
      activeAccountId: UserDefaults.standard.string(forKey: "activeAccountId").flatMap(
        UUID.init(uuidString:)))
  }
}
