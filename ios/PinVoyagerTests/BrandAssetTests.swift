import UIKit
import XCTest

@MainActor
final class BrandAssetTests: XCTestCase {
  func testAccountBrandIconLoadsDistinctLightAndDarkArtwork() throws {
    let light = try XCTUnwrap(
      UIImage(
        named: "BrandIcon",
        in: .main,
        compatibleWith: UITraitCollection(userInterfaceStyle: .light)
      )
    )
    let dark = try XCTUnwrap(
      UIImage(
        named: "BrandIcon",
        in: .main,
        compatibleWith: UITraitCollection(userInterfaceStyle: .dark)
      )
    )
    XCTAssertEqual(light.size, CGSize(width: 1024, height: 1024))
    XCTAssertEqual(dark.size, light.size)
    XCTAssertNotEqual(try XCTUnwrap(light.pngData()), try XCTUnwrap(dark.pngData()))
  }
}
