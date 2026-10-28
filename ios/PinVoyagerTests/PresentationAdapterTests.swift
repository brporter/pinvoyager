import UIKit
import XCTest

@testable import PinVoyager

@MainActor final class PresentationAdapterTests: XCTestCase {
  func testAuthenticationAnchorTracksTheInvokingWindowAndReleasesItAfterRemoval() {
    let context = WindowContext()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 640))
    let probe = WindowProbe(windowContext: context)
    window.addSubview(probe)
    XCTAssertTrue(context.anchor() === window)
    XCTAssertFalse(probe.isUserInteractionEnabled)
    probe.removeFromSuperview()
    XCTAssertNil(context.window)
    XCTAssertFalse(context.anchor() === window)
  }

  func testFilesExportDelegateReportsTheChosenDestinationAndCancellation() throws {
    let file = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".zip")
    try Data("archive fixture".utf8).write(to: file)
    defer { try? FileManager.default.removeItem(at: file) }
    let picker = UIDocumentPickerViewController(forExporting: [file], asCopy: true)
    var completions: [URL?] = []
    let coordinator = ExportFilesPicker.Coordinator { completions.append($0) }
    let destination = URL.temporaryDirectory.appendingPathComponent("selected.zip")
    coordinator.documentPicker(picker, didPickDocumentsAt: [destination])
    coordinator.documentPickerWasCancelled(picker)
    coordinator.documentPicker(picker, didPickDocumentsAt: [])
    XCTAssertEqual(completions.count, 3)
    XCTAssertEqual(completions[0], destination)
    XCTAssertNil(completions[1])
    XCTAssertNil(completions[2])
  }

  func testDoubleTapRestoresThePhotoScaleAndReenablesPagingThroughTheZoomCallback() async {
    let coordinator = ZoomablePhoto.Coordinator()
    let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
    let photo = UIImageView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
    scroll.addSubview(photo)
    coordinator.imageView = photo
    scroll.delegate = coordinator
    scroll.minimumZoomScale = 1
    scroll.maximumZoomScale = 5
    var zoomed = false
    coordinator.onZoomChanged = { zoomed = $0 }
    scroll.zoomScale = 2
    coordinator.scrollViewDidZoom(scroll)
    XCTAssertTrue(zoomed)
    let gesture = UITapGestureRecognizer()
    scroll.addGestureRecognizer(gesture)
    coordinator.reset(gesture)
    let reset = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "zoomScale == 1"), object: scroll)
    await fulfillment(of: [reset], timeout: 2)
    coordinator.scrollViewDidZoom(scroll)
    XCTAssertEqual(scroll.zoomScale, 1)
    XCTAssertFalse(zoomed)
    XCTAssertTrue(coordinator.viewForZooming(in: scroll) === photo)
  }
}
