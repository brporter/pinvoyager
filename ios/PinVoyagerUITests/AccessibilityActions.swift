import XCTest

extension XCTestCase {
  @MainActor func assertGalleryFillsWindow(
    _ app: XCUIApplication, context: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    let navigation = app.navigationBars.matching(
      NSPredicate(format: "identifier MATCHES %@", "[0-9]+ of [0-9]+")
    ).firstMatch
    XCTAssertTrue(navigation.waitForExistence(timeout: 5), file: file, line: line)
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = "Gallery size — \(context)"
    screenshot.lifetime = .keepAlways
    add(screenshot)
    let window = app.windows.firstMatch.frame
    XCTAssertGreaterThanOrEqual(
      navigation.frame.width, window.width * 0.95,
      "The gallery must use the window's width.", file: file, line: line)
    XCTAssertLessThanOrEqual(
      navigation.frame.minY, window.minY + window.height * 0.08,
      "The gallery must open at the top of the window.", file: file, line: line)
    XCTAssertGreaterThanOrEqual(
      app.buttons["Next"].frame.midY, window.minY + window.height * 0.85,
      "Gallery controls must extend to the bottom of the window.", file: file, line: line)
  }
}

extension XCUIElement {
  /// Scroll beside embedded maps so their pan gesture does not consume the drag.
  @MainActor func reveal(
    _ element: XCUIElement, swipeUp: Bool = true, maximumSwipes: Int = 8,
    within visibleFrame: CGRect? = nil
  ) {
    for _ in 0..<maximumSwipes {
      let bounds = frame
      let visible = visibleFrame ?? bounds
      let target = element.exists ? element.frame : nil
      if let target, visible.contains(target), element.isHittable {
        return
      }
      let forward = target.map { $0.midY > visible.midY } ?? swipeUp
      let distance = min(
        bounds.height / 2, max(80, target.map { abs($0.midY - visible.midY) } ?? bounds.height / 2))
      let start: CGFloat = forward ? 0.8 : 0.25
      let end = start + (forward ? -1 : 1) * distance / bounds.height
      coordinate(withNormalizedOffset: CGVector(dx: 0.04, dy: start)).press(
        forDuration: 0.05,
        thenDragTo: coordinate(withNormalizedOffset: CGVector(dx: 0.04, dy: end)))
    }
  }
}

extension XCUIApplication {
  @MainActor func selectFirstSystemPhoto(requiresConfirmation: Bool = true) {
    let photo = images.matching(NSPredicate(format: "label BEGINSWITH[c] %@", "Photo,")).firstMatch
    XCTAssertTrue(photo.waitForExistence(timeout: 10), debugDescription)
    photo.tap()
    let add = navigationBars["Photos"].buttons.matching(
      NSPredicate(format: "label == %@ OR label BEGINSWITH %@", "Done", "Add")
    ).firstMatch
    if requiresConfirmation {
      XCTAssertTrue(add.waitForExistence(timeout: 5), debugDescription)
      add.tap()
    } else if add.exists {
      add.tap()
    }
  }
}
