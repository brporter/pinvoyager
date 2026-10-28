import XCTest

@testable import PinVoyager

@MainActor final class GalleryNavigationTests: XCTestCase {
  func testRefreshKeepsTheSelectedPhotoIdentityAndFallsBackToItsNearestSurvivingPosition() {
    let ids = (0..<4).map { _ in UUID() }
    let navigation = GalleryNavigation(ids: ids, selectedId: ids[2])
    XCTAssertEqual(navigation.selectedId, ids[2])
    navigation.zoomed = true
    navigation.refresh([ids[3], ids[2], ids[0], ids[1]])
    XCTAssertEqual(navigation.selectedId, ids[2])
    XCTAssertTrue(navigation.zoomed)
    navigation.refresh([ids[3], ids[0], ids[1]])
    XCTAssertEqual(navigation.selectedId, ids[0])
    XCTAssertFalse(navigation.zoomed)
    navigation.move(-1)
    XCTAssertEqual(navigation.selectedId, ids[3])
    navigation.move(-1)
    XCTAssertEqual(navigation.selectedId, ids[3])
    navigation.select(UUID())
    XCTAssertEqual(navigation.selectedId, ids[3])
    navigation.move(2)
    XCTAssertEqual(navigation.selectedId, ids[1])
    navigation.refresh([])
    XCTAssertNil(navigation.selectedId)
    navigation.move(1)
    navigation.refresh([ids[1]])
    XCTAssertEqual(navigation.selectedId, ids[1])
  }
}
