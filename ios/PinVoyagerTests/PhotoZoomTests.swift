import SwiftUI
import UIKit
import XCTest

@testable import PinVoyager

@MainActor final class PhotoZoomTests: XCTestCase {
  func testZoomRequestsHigherResolutionOnlyOnce() {
    let coordinator = ZoomablePhoto.Coordinator()
    var requests = 0
    var zoomStates: [Bool] = []
    coordinator.onZoomChanged = { zoomStates.append($0) }
    coordinator.onZoom = { requests += 1 }
    let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
    let image = UIImageView(frame: CGRect(x: 0, y: 0, width: 400, height: 400))
    scroll.addSubview(image)
    scroll.contentSize = image.bounds.size
    scroll.delegate = coordinator
    coordinator.imageView = image
    scroll.minimumZoomScale = 1
    scroll.maximumZoomScale = 5
    scroll.zoomScale = 1
    coordinator.scrollViewDidZoom(scroll)
    XCTAssertEqual(requests, 0)
    scroll.zoomScale = 2
    coordinator.scrollViewDidZoom(scroll)
    coordinator.scrollViewDidZoom(scroll)
    XCTAssertEqual(requests, 1)
    XCTAssertEqual(zoomStates, [true])
    scroll.zoomScale = 1
    coordinator.scrollViewDidZoom(scroll)
    XCTAssertEqual(zoomStates, [true, false])
  }
}
