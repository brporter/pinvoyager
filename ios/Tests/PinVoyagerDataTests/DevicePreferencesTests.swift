import Foundation
import PinVoyagerDomain
import Testing

@testable import PinVoyagerData

@Test func signedOutDevicePreferencesSurviveRestartWithoutSharingAnotherNamespace() async throws {
  let suite = "pinvoyager.preferences.test.\(UUID())"
  let other = "pinvoyager.preferences.test.\(UUID())"
  defer {
    UserDefaults.standard.removePersistentDomain(forName: suite)
    UserDefaults.standard.removePersistentDomain(forName: other)
  }
  let preferences = UserDefaultsDevicePreferences(suiteName: suite)
  #expect(try await preferences.load() == SyncPolicy())
  var policy = SyncPolicy()
  policy.wifiOnly = false
  policy.allowsConstrained = true
  policy.backgroundPreparation = false
  try await preferences.save(policy)
  let reopened = UserDefaultsDevicePreferences(suiteName: suite)
  #expect(try await reopened.load() == policy)
  #expect(try await UserDefaultsDevicePreferences(suiteName: other).load() == SyncPolicy())
}

@Test func damagedDevicePreferencesRemainRecoverableAndDoNotInventMorePermissiveDefaults()
  async throws
{
  let suite = "pinvoyager.preferences.test.\(UUID())"
  defer { UserDefaults.standard.removePersistentDomain(forName: suite) }
  let raw = try #require(UserDefaults(suiteName: suite))
  raw.set(Data("damaged policy".utf8), forKey: UserDefaultsDevicePreferences.policyKey)
  await #expect(throws: DecodingError.self) {
    try await UserDefaultsDevicePreferences(suiteName: suite).load()
  }
  #expect(raw.data(forKey: UserDefaultsDevicePreferences.policyKey) == Data("damaged policy".utf8))
  try await UserDefaultsDevicePreferences(suiteName: suite).save(SyncPolicy())
  #expect(try await UserDefaultsDevicePreferences(suiteName: suite).load().wifiOnly)
}

@Test func concurrentAccountPreferencesNeverRestoreAnOlderToggleSnapshot() async throws {
  let state = unlockedWorkspace()
  let store = try CoreDataWorkspaceStore(url: nil)
  try await store.insert(state)
  let repository = LocalRepository(store: store)
  async let wifi: Void = repository.setPreference(
    accountId: state.account.id, preference: .wifiOnly, value: false)
  async let constrained: Void = repository.setPreference(
    accountId: state.account.id, preference: .allowsConstrained, value: true)
  async let background: Void = repository.setPreference(
    accountId: state.account.id, preference: .backgroundPreparation, value: false)
  async let paused: Void = repository.setPreference(
    accountId: state.account.id, preference: .paused, value: true)
  async let originals: Void = repository.setPreference(
    accountId: state.account.id, preference: .keepOriginals, value: false)
  _ = try await (wifi, constrained, background, paused, originals)
  let saved = try await store.read(accountId: state.account.id)
  #expect(!saved.policy.wifiOnly)
  #expect(saved.policy.allowsConstrained)
  #expect(!saved.policy.backgroundPreparation)
  #expect(saved.policy.paused)
  #expect(!saved.policy.keepOriginals)
  #expect(saved.account.generation == 3)
}
