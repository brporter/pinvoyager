import PinVoyagerData
import PinVoyagerDomain
import XCTest

@MainActor struct FixtureLaunch {
  var workspace: Workspace
  var identities: [LinkedIdentity] = []
  var deletions: [PendingAccountDeletion] = []
  var replies: [String: [String: String]] = [:]
  init() throws {
    let me = try APIJSON.decoder().decode(AccountResponse.self, from: Self.fixture("me"))
    workspace = Workspace(account: LocalAccount(profile: me.user))
    identities = me.identities
    var capabilities = try XCTUnwrap(
      try JSONSerialization.jsonObject(with: Self.fixture("native_capabilities")) as? [String: Any])
    capabilities["native_protocol_versions"] = []
    capabilities["providers"] = ["apple", "google", "microsoft"]
    reply(
      "/api/client-capabilities", body: try JSONSerialization.data(withJSONObject: capabilities))
    reply("/api/me", body: try Self.fixture("me"))
  }
  static func fixture(_ name: String) throws -> Data {
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent()
    return try Data(
      contentsOf: root.appendingPathComponent("web/src/api/__fixtures__/\(name).json"))
  }
  mutating func reply(
    _ path: String, body: Data, status: Int = 200, headers: [String: String] = [:]
  ) {
    var response = ["body": body.base64EncodedString(), "status": String(status)]
    for (name, value) in headers { response["header." + name] = value }
    replies[path.lowercased()] = response
  }
  func deletion(status: AccountDeletionStatus) throws -> PendingAccountDeletion {
    try JSONDecoder().decode(
      PendingAccountDeletion.self,
      from: JSONSerialization.data(withJSONObject: [
        "id": UUID().uuidString, "accountId": workspace.account.id.uuidString,
        "statusCapability": "synthetic-status-capability-only-used-in-tests",
        "proofToken": "synthetic-proof",
        "status": status.rawValue,
        "expiresAt": Date().addingTimeInterval(600).timeIntervalSinceReferenceDate,
      ]))
  }
  func launch(arguments: [String] = []) throws -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--ui-testing"] + arguments
    app.launchEnvironment["PINVOYAGER_TEST_WORKSPACE"] = String(
      decoding: try JSONEncoder().encode(workspace), as: UTF8.self)
    app.launchEnvironment["PINVOYAGER_TEST_DELETIONS"] = String(
      decoding: try JSONEncoder().encode(deletions), as: UTF8.self)
    app.launchEnvironment["PINVOYAGER_TEST_IDENTITIES"] = String(
      decoding: try APIJSON.encode(identities), as: UTF8.self)
    app.launchEnvironment["PINVOYAGER_TEST_RESPONSES"] = String(
      decoding: try JSONSerialization.data(withJSONObject: replies), as: UTF8.self)
    app.launch()
    return app
  }
}
