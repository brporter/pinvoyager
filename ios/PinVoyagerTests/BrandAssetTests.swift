import UIKit
import XCTest

@MainActor
final class BrandAssetTests: XCTestCase {
  func testProviderLogosLoadAsFullColorOriginalArtwork() throws {
    XCTAssertNotNil(UIFont(name: "GoogleSans-Regular", size: 17))
    for name in ["AppleSignIn", "GoogleSignIn", "MicrosoftSignIn"] {
      let image = try XCTUnwrap(UIImage(named: name))
      XCTAssertGreaterThan(image.size.width, 0)
      XCTAssertEqual(image.renderingMode, .alwaysOriginal)
    }
  }

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
