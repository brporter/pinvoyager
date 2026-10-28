import CryptoKit
import PinVoyagerDomain
import XCTest

extension FixtureLaunch {
  mutating func provideOwnerDownload(voyage: LocalVoyage, photo: LocalPhoto, bytes: Data) throws {
    let remoteVoyage = try XCTUnwrap(voyage.serverId)
    let remotePhoto = try XCTUnwrap(photo.serverId)
    let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    let variants = ["thumb", "display", "full"].map { kind in
      [
        "kind": kind, "url": "/api/photos/\(remotePhoto)/offline/\(kind)",
        "byte_count": bytes.count, "sha256": digest,
      ] as [String: Any]
    }
    let manifest: [String: Any] = [
      "voyage_id": remoteVoyage.uuidString, "owner_id": workspace.account.id.uuidString,
      "revision": voyage.revision ?? "1", "title": voyage.fields.title,
      "description": voyage.fields.description.map { $0 as Any } ?? NSNull(),
      "granularity": voyage.fields.granularity.rawValue,
      "published": voyage.confirmedPublished, "retention_permission": "owner_offline_copy_v1",
      "photos": [
        [
          "id": remotePhoto.uuidString, "position": 0, "width": 120, "height": 80,
          "latitude": photo.coordinate.map { $0.latitude as Any } ?? NSNull(),
          "longitude": photo.coordinate.map { $0.longitude as Any } ?? NSNull(),
          "variants": variants,
        ]
      ],
    ]
    reply(
      "/api/voyages/\(remoteVoyage)/offline-manifest",
      body: try JSONSerialization.data(withJSONObject: manifest))
    for kind in ["thumb", "display", "full"] {
      reply(
        "/api/photos/\(remotePhoto)/offline/\(kind)", body: bytes,
        headers: ["x-pinvoyager-retention": "owner_offline_copy_v1"])
      reply("/api/photos/\(remotePhoto)/\(kind)", body: bytes)
    }
  }
}
