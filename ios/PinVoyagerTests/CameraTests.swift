@preconcurrency import AVFoundation
import CoreLocation
import ImageIO
import PinVoyagerDomain
import UIKit
import XCTest

@testable import PinVoyager

private actor CaptureStub: PhotoCapturing {
  nonisolated let box = CaptureSessionBox()
  var bytes: Data
  var fails = false
  var starts = 0
  var stops = 0
  var captures = 0
  init(bytes: Data) { self.bytes = bytes }
  func configure() throws { if fails { throw CameraError.unavailable } }
  func start() { starts += 1 }
  func stop() { stops += 1 }
  func flip() throws { if fails { throw CameraError.unavailable } }
  func capture(flash: Bool) throws -> Data {
    captures += 1
    if fails { throw CameraError.unavailable }
    return bytes
  }
  func setFailure(_ value: Bool) { fails = value }
}
@MainActor private final class CameraLocationStub: CameraLocationProviding {
  var current: CLLocation?
  var starts = 0
  var stops = 0
  func start() { starts += 1 }
  func stop() { stops += 1 }
}
@MainActor final class CameraTests: XCTestCase {
  func testPermissionDenialAndEngineFailureKeepTheImportAlternativeAvailable() async {
    let engine = CaptureStub(bytes: Data())
    let controller = CameraController(
      engine: engine, location: CameraLocationStub(), permission: { false })
    await controller.start()
    XCTAssertEqual(controller.error, CameraError.permission.localizedDescription)
    let starts = await engine.starts
    XCTAssertEqual(starts, 0)
    let allowed = CameraController(
      engine: engine, location: CameraLocationStub(), permission: { true })
    await engine.setFailure(true)
    await allowed.start()
    XCTAssertEqual(allowed.error, CameraError.unavailable.localizedDescription)
    await allowed.takePhoto()
    XCTAssertNil(allowed.photo)
    XCTAssertFalse(allowed.working)
  }
  func testCameraUsesOnlyRecentAccurateLocationsAndCanCaptureWithoutLocation() async throws {
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8)).jpegData(
      withCompressionQuality: 0.8
    ) { context in
      UIColor.systemTeal.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
    }
    for (age, accuracy, included) in [
      (0.0, 10.0, true), (60.0, 10.0, false), (0.0, -1.0, false), (0.0, 1001.0, false),
    ] {
      let engine = CaptureStub(bytes: bytes)
      let location = CameraLocationStub()
      location.current = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: -12.34, longitude: -45.67), altitude: 0,
        horizontalAccuracy: accuracy, verticalAccuracy: -1,
        timestamp: Date().addingTimeInterval(-age))
      let controller = CameraController(engine: engine, location: location, permission: { true })
      await controller.start()
      controller.includeLocation = true
      controller.updateLocation()
      XCTAssertEqual(location.starts, 1)
      await controller.takePhoto()
      XCTAssertEqual(controller.locationIncluded, included)
      XCTAssertNotNil(controller.photo)
      let source = try XCTUnwrap(
        CGImageSourceCreateWithData(try XCTUnwrap(controller.photo) as CFData, nil))
      let properties = try XCTUnwrap(
        CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
      if included {
        let gps = try XCTUnwrap(properties[kCGImagePropertyGPSDictionary] as? [CFString: Any])
        XCTAssertEqual(gps[kCGImagePropertyGPSLatitudeRef] as? String, "S")
        XCTAssertEqual(gps[kCGImagePropertyGPSLongitudeRef] as? String, "W")
      } else {
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary])
      }
      controller.includeLocation = false
      controller.updateLocation()
      XCTAssertEqual(location.stops, 1)
      await controller.stop()
      let stops = await engine.stops
      XCTAssertEqual(stops, 1)
    }
    XCTAssertThrowsError(
      try CameraController.attachLocation(
        Coordinate(latitude: 1, longitude: 2), to: Data("invalid".utf8)))
    XCTAssertEqual(try CameraController.attachLocation(nil, to: bytes), bytes)
  }
}

extension CameraTests {
  func testSavingACaptureRejectsDuplicateTapsAndKeepsThePhotoAfterFailure() async throws {
    let bytes = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8)).jpegData(
      withCompressionQuality: 0.8
    ) { context in
      UIColor.systemTeal.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
    }
    let controller = CameraController(
      engine: CaptureStub(bytes: bytes), location: CameraLocationStub(), permission: { true })
    controller.photo = bytes
    let started = expectation(
      description: "Captured file remains available while its import is being saved")
    var gate: CheckedContinuation<Void, Never>?
    var path: URL?
    let pending = Task {
      await controller.save { url in
        path = url
        XCTAssertEqual(try Data(contentsOf: url), bytes)
        await withCheckedContinuation { continuation in
          gate = continuation
          started.fulfill()
        }
      }
    }
    await fulfillment(of: [started], timeout: 2)
    XCTAssertTrue(controller.working)
    let duplicate = await controller.save { _ in
      XCTFail("A second tap must not import the photograph twice")
    }
    XCTAssertFalse(duplicate)
    gate?.resume()
    let accepted = await pending.value
    XCTAssertTrue(accepted)
    XCTAssertFalse(controller.working)
    XCTAssertFalse(FileManager.default.fileExists(atPath: try XCTUnwrap(path).path))
    let failed = await controller.save { _ in throw CameraError.unavailable }
    XCTAssertFalse(failed)
    XCTAssertEqual(controller.photo, bytes)
    XCTAssertNotNil(controller.error)
  }
}
