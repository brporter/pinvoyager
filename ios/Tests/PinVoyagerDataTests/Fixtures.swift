import Foundation

private final class FixtureBundle: NSObject {}

enum Fixtures {
  static func directory(_ name: String) -> URL {
    #if SWIFT_PACKAGE
      let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
      return root.appendingPathComponent(
        name == "api" ? "web/src/api/__fixtures__" : "crates/api/tests/fixtures")
    #else
      return Bundle(for: FixtureBundle.self).resourceURL!.appendingPathComponent(
        name == "api" ? "__fixtures__" : "fixtures")
    #endif
  }
}
