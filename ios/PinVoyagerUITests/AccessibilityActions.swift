import PinVoyagerDomain
import XCTest

class UITestCase: XCTestCase {
  override func setUpWithError() throws {
    try super.setUpWithError()
    continueAfterFailure = false
  }
}

extension XCTestCase {
  @MainActor
  func assertGalleryFillsWindow(
    _ app: XCUIApplication,
    context: String,
    file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let navigation = app.navigationBars.matching(
      NSPredicate(format: "identifier MATCHES %@", "[0-9]+ of [0-9]+")
    ).firstMatch
    XCTAssertTrue(navigation.waitUntilExists(timeout: 5), file: file, line: line)
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = "Gallery size — \(context)"
    screenshot.lifetime = .keepAlways
    add(screenshot)
    let window = app.windows.firstMatch.frame
    XCTAssertGreaterThanOrEqual(
      navigation.frame.width,
      window.width * 0.95,
      "The gallery must use the window's width.",
      file: file,
      line: line
    )
    XCTAssertLessThanOrEqual(
      navigation.frame.minY,
      window.minY + window.height * 0.08,
      "The gallery must open at the top of the window.",
      file: file,
      line: line
    )
    XCTAssertGreaterThanOrEqual(
      app.buttons["Next"].frame.midY,
      window.minY + window.height * 0.85,
      "Gallery controls must extend to the bottom of the window.",
      file: file,
      line: line
    )
  }
}

extension XCUIElement {
  /// Check immediately before starting XCTest's polling wait.
  @MainActor
  func waitUntilExists(timeout: TimeInterval) -> Bool {
    exists || waitForExistence(timeout: timeout)
  }

  @MainActor
  func waitUntilHittable(timeout: TimeInterval) -> Bool {
    (exists && isHittable) || wait(for: \.isHittable, toEqual: true, timeout: timeout)
  }

  /// Scroll beside embedded maps so their pan gesture does not consume the drag.
  @MainActor
  func reveal(
    _ element: XCUIElement,
    swipeUp: Bool = true,
    maximumSwipes: Int = 8,
    within visibleFrame: CGRect? = nil,
    file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let bounds = frame
    let keyboard = (self as? XCUIApplication)?.keyboards.firstMatch
    var keyboardWasVisible = keyboard?.exists ?? false
    var previousTarget: CGRect?
    var stationarySwipes = 0
    for swipe in 0...maximumSwipes {
      var visible = visibleFrame ?? bounds
      if keyboardWasVisible, let keyboard {
        keyboardWasVisible = keyboard.exists
        if keyboardWasVisible {
          visible.size.height = max(0, min(visible.maxY, keyboard.frame.minY) - visible.minY)
        }
      }
      let target = element.exists ? element.frame : nil
      if let target, visible.contains(target), element.isHittable {
        return
      }
      if let target, !target.isEmpty, target == previousTarget {
        stationarySwipes += 1
      } else {
        stationarySwipes = 0
      }
      guard swipe < maximumSwipes, stationarySwipes < 3, bounds.height > 0, visible.height > 0
      else {
        XCTFail(
          "Could not reveal \(element): scrolling exhausted or stopped moving.",
          file: file,
          line: line
        )
        return
      }
      previousTarget = target
      let forward = target.map { $0.midY > visible.midY } ?? swipeUp
      let distance = min(
        visible.height / 2,
        max(80, target.map { abs($0.midY - visible.midY) } ?? visible.height / 2)
      )
      let start =
        (visible.minY - bounds.minY + visible.height * (forward ? 0.8 : 0.25))
        / bounds.height
      let end = start + (forward ? -1 : 1) * distance / bounds.height
      coordinate(withNormalizedOffset: CGVector(dx: 0.04, dy: start)).press(
        forDuration: 0.05,
        thenDragTo: coordinate(withNormalizedOffset: CGVector(dx: 0.04, dy: end))
      )
    }
  }
}

extension XCUIApplication {
  @MainActor
  func launchFixture(tab: AppTab = .discover) {
    launchEnvironment["PINVOYAGER_TEST_TAB"] = tab.rawValue
    launch()
  }

  @MainActor
  func showDiscover(file: StaticString = #filePath, line: UInt = #line) throws {
    let tab = tabBars.buttons["Discover"]
    try XCTUnwrap(
      tab.waitUntilExists(timeout: 10) ? tab : nil,
      "The Discover tab must be available.",
      file: file,
      line: line
    ).tap()

    let root = navigationBars["Discover"]
    for _ in 0..<8 {
      if root.exists { break }
      let back = navigationBars.buttons.element(boundBy: 0)
      try XCTUnwrap(
        back.waitUntilExists(timeout: 5) ? back : nil,
        "A public destination must offer navigation back to Discover.",
        file: file,
        line: line
      ).tap()
    }
    _ = try XCTUnwrap(
      root.waitUntilExists(timeout: 5) ? root : nil,
      "Discover setup must return to the root navigation screen.",
      file: file,
      line: line
    )
    let recent = buttons["Recent"]
    try XCTUnwrap(
      recent.waitUntilExists(timeout: 5) ? recent : nil,
      "Discover must offer its Recent list.",
      file: file,
      line: line
    ).tap()
  }

  @MainActor
  func selectFirstSystemPhoto(requiresConfirmation: Bool = true) {
    let photo = images.matching(NSPredicate(format: "label BEGINSWITH[c] %@", "Photo,")).firstMatch
    XCTAssertTrue(photo.waitUntilExists(timeout: 10), debugDescription)
    let photoFrame = photo.frame
    XCTAssertFalse(photoFrame.isEmpty)
    XCTAssertTrue(windows.firstMatch.frame.contains(photoFrame))
    // iOS 26.4.1's remote Photos view reports no hit point for this visible image.
    // Select its center; the confirmation and import assertions still verify the action.
    photo.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let add = navigationBars["Photos"].buttons.matching(
      NSPredicate(format: "label == %@ OR label BEGINSWITH %@", "Done", "Add")
    ).firstMatch
    if requiresConfirmation {
      XCTAssertTrue(add.waitUntilExists(timeout: 5), debugDescription)
      add.tap()
    } else if add.exists {
      add.tap()
    }
  }
}
