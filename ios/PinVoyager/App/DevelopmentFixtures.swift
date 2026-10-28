#if DEBUG
  import Foundation
  import CoreLocation
  import UIKit
  import PinVoyagerData
  import PinVoyagerDomain

  struct DevelopmentTransport: HTTPTransport {
    func send(_ request: URLRequest) async throws -> HTTPResponse {
      if let encoded = ProcessInfo.processInfo.environment["PINVOYAGER_TEST_RESPONSES"],
        let data = encoded.data(using: .utf8),
        let replies = try JSONSerialization.jsonObject(with: data) as? [String: [String: String]],
        let path = request.url?.path.lowercased(),
        let reply = replies[path]
          ?? replies.first(where: {
            $0.key.hasSuffix("/*") && path.hasPrefix(String($0.key.dropLast()))
          })?.value,
        let body = reply["body"].flatMap({ Data(base64Encoded: $0) }),
        let status = reply["status"].flatMap(Int.init)
      {
        let headers = Dictionary(
          reply.compactMap { name, value -> (String, String)? in
            guard name.hasPrefix("header.") else { return nil }
            return (String(name.dropFirst(7)), value)
          }, uniquingKeysWith: { _, last in last })
        return HTTPResponse(status: status, headers: headers, body: body)
      }
      if request.url?.path == "/api/client-capabilities" {
        return HTTPResponse(status: 404, body: Data())
      }
      if request.url?.path == "/api/voyages" {
        return HTTPResponse(status: 200, body: Data("{\"voyages\":[],\"next_before\":null}".utf8))
      }
      return HTTPResponse(status: 404, body: Data())
    }
  }

  @MainActor
  enum DevelopmentFixtures {
    static func seed(_ model: AppModel) async throws {
      let args = ProcessInfo.processInfo.arguments
      await model.restore(accountId: nil)
      if args.contains("--offline-fixture") { model.changeNetwork(.offline) }
      if let encoded = ProcessInfo.processInfo.environment["PINVOYAGER_TEST_CAMERA"],
        let bytes = Data(base64Encoded: encoded)
      {
        model.makeCamera = {
          CameraController(
            engine: FixedPhotoCapture(bytes: bytes), location: FixedCameraLocation(),
            permission: { true })
        }
      }
      model.capabilityChecked = true
      if ProcessInfo.processInfo.environment["PINVOYAGER_TEST_RESPONSES"] != nil {
        await model.loadDiscover(refresh: true)
      }
      if let encoded = ProcessInfo.processInfo.environment["PINVOYAGER_TEST_WORKSPACE"],
        let data = encoded.data(using: .utf8)
      {
        let state = try JSONDecoder().decode(Workspace.self, from: data)
        try await model.store.insert(state)
        try await model.credentials.save(
          SessionCredential(accountId: state.account.id, token: "synthetic-ui-test-session"))
        await model.restore(accountId: state.account.id)
        if let encoded = ProcessInfo.processInfo.environment["PINVOYAGER_TEST_IDENTITIES"],
          let data = encoded.data(using: .utf8)
        {
          model.identities = try APIJSON.decoder().decode([LinkedIdentity].self, from: data)
        }
        if let encoded = ProcessInfo.processInfo.environment["PINVOYAGER_TEST_DELETIONS"],
          let data = encoded.data(using: .utf8)
        {
          for record in try JSONDecoder().decode([PendingAccountDeletion].self, from: data) {
            try await model.accountDeletion.recovery.save(record)
          }
          await model.reloadDeletionRecovery()
        }
        await model.checkCapabilities()
        return
      }
      guard args.contains("--signed-in-fixture") else { return }
      let account = LocalAccount(profile: UserProfile(id: UUID(), displayName: "test-traveler"))
      try await model.store.insert(Workspace(account: account))
      await model.restore(accountId: account.id)
      guard args.contains("--populated-fixture"), let vault = model.vault else { return }
      let id = try await model.create(
        VoyageFields(title: "Winter in Hokkaido", description: "A quiet walk through fresh snow."))
      let dimension = args.contains("--large-photo-fixture") ? 2048.0 : 80.0
      let data = UIGraphicsImageRenderer(size: CGSize(width: dimension, height: dimension))
        .jpegData(
          withCompressionQuality: 0.8
        ) { context in
          UIColor.systemTeal.setFill()
          context.fill(CGRect(x: 0, y: 0, width: dimension, height: dimension))
        }
      let source = URL.temporaryDirectory.appendingPathComponent("Fixture.jpg")
      try data.write(to: source)
      defer { try? FileManager.default.removeItem(at: source) }
      let photo = try await PhotoPreparer(vault: vault).prepare(
        source, accountId: account.id, voyageId: id, maxBytes: 25 * 1_024 * 1_024)
      try await model.repository.addPhoto(accountId: account.id, photo: photo)
      await model.reloadWorkspace()
    }
  }
  private actor FixedPhotoCapture: PhotoCapturing {
    nonisolated let box = CaptureSessionBox()
    let bytes: Data
    init(bytes: Data) { self.bytes = bytes }
    func configure() {}
    func start() {}
    func stop() {}
    func flip() {}
    func capture(flash: Bool) -> Data { bytes }
  }
  @MainActor private final class FixedCameraLocation: CameraLocationProviding {
    var current: CLLocation? { nil }
    func start() {}
    func stop() {}
  }
#endif
