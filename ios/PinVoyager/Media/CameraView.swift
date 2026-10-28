@preconcurrency import AVFoundation
import CoreLocation
import ImageIO
import Observation
import PinVoyagerDomain
import SwiftUI
import UniformTypeIdentifiers

// AVFoundation owns synchronization; only the capture actor mutates the session.
final class CaptureSessionBox: @unchecked Sendable { let session = AVCaptureSession() }

protocol PhotoCapturing: Sendable {
  var box: CaptureSessionBox { get }
  func configure() async throws
  func start() async
  func stop() async
  func flip() async throws
  func capture(flash: Bool) async throws -> Data
}

@MainActor protocol CameraLocationProviding: AnyObject {
  var current: CLLocation? { get }
  func start()
  func stop()
}

@MainActor
private final class CameraLocation: NSObject, CameraLocationProviding, CLLocationManagerDelegate {
  private let manager = CLLocationManager()
  var current: CLLocation? { manager.location }
  override init() {
    super.init()
    manager.delegate = self
    manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
  }
  func start() {
    manager.requestWhenInUseAuthorization()
    manager.startUpdatingLocation()
  }
  func stop() { manager.stopUpdatingLocation() }
}

actor CaptureEngine: PhotoCapturing {
  nonisolated let box = CaptureSessionBox()
  private let output = AVCapturePhotoOutput()
  private var delegate: StillPhotoDelegate?
  private var position = AVCaptureDevice.Position.back

  func configure() throws {
    let session = box.session
    session.beginConfiguration()
    defer { session.commitConfiguration() }
    session.sessionPreset = .photo
    for input in session.inputs { session.removeInput(input) }
    guard
      let camera = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: position)
    else { throw CameraError.unavailable }
    let input = try AVCaptureDeviceInput(device: camera)
    guard session.canAddInput(input) else { throw CameraError.unavailable }
    session.addInput(input)
    if session.outputs.isEmpty {
      guard session.canAddOutput(output) else { throw CameraError.unavailable }
      session.addOutput(output)
    }
  }
  func start() { if !box.session.isRunning { box.session.startRunning() } }
  func stop() { if box.session.isRunning { box.session.stopRunning() } }
  func flip() throws {
    position = position == .back ? .front : .back
    try configure()
  }
  func capture(flash: Bool) async throws -> Data {
    let settings = AVCapturePhotoSettings()
    if output.supportedFlashModes.contains(.on) { settings.flashMode = flash ? .on : .off }
    return try await withCheckedThrowingContinuation { continuation in
      let delegate = StillPhotoDelegate { result in continuation.resume(with: result) }
      self.delegate = delegate
      output.capturePhoto(with: settings, delegate: delegate)
    }
  }
}

private final class StillPhotoDelegate: NSObject, AVCapturePhotoCaptureDelegate, Sendable {
  let completion: @Sendable (Result<Data, any Error>) -> Void
  init(completion: @escaping @Sendable (Result<Data, any Error>) -> Void) {
    self.completion = completion
  }
  func photoOutput(
    _ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto,
    error: (any Error)?
  ) {
    if let error {
      completion(.failure(error))
    } else if let bytes = photo.fileDataRepresentation() {
      completion(.success(bytes))
    } else {
      completion(.failure(CameraError.unavailable))
    }
  }
}

enum CameraError: Error, LocalizedError {
  case unavailable, permission
  var errorDescription: String? {
    self == .permission
      ? "Camera access is off. Enable it in Settings or choose photos instead."
      : "The camera is unavailable. You can choose photos instead."
  }
}

@MainActor @Observable
final class CameraController {
  let engine: any PhotoCapturing
  private let location: any CameraLocationProviding
  private let permission: @Sendable () async -> Bool
  var error: String?
  var photo: Data?
  var working = false
  var includeLocation = false
  var locationIncluded = false
  var flash = false
  init(
    engine: any PhotoCapturing = CaptureEngine(), location: (any CameraLocationProviding)? = nil,
    permission: @escaping @Sendable () async -> Bool = {
      await AVCaptureDevice.requestAccess(for: .video)
    }
  ) {
    self.engine = engine
    self.location = location ?? CameraLocation()
    self.permission = permission
  }
  func start() async {
    guard await permission() else {
      error = CameraError.permission.localizedDescription
      return
    }
    do {
      try await engine.configure()
      await engine.start()
    } catch { self.error = error.localizedDescription }
  }
  func updateLocation() {
    if includeLocation {
      location.start()
    } else {
      location.stop()
    }
  }
  func stop() async {
    location.stop()
    await engine.stop()
  }
  func flip() async {
    do { try await engine.flip() } catch { self.error = error.localizedDescription }
  }
  func takePhoto() async {
    guard !working else { return }
    working = true
    defer { working = false }
    do {
      error = nil
      let location = includeLocation ? location.current : nil
      let acceptable = location.flatMap { location -> Coordinate? in
        guard location.horizontalAccuracy >= 0, location.horizontalAccuracy <= 1_000,
          abs(location.timestamp.timeIntervalSinceNow) <= 30
        else { return nil }
        return Coordinate(
          latitude: location.coordinate.latitude, longitude: location.coordinate.longitude)
      }
      let bytes = try await engine.capture(flash: flash)
      photo = try await Task.detached { try Self.attachLocation(acceptable, to: bytes) }.value
      locationIncluded = acceptable != nil
    } catch { self.error = error.localizedDescription }
  }
  func save(using importPhoto: @MainActor (URL) async throws -> Void) async -> Bool {
    guard let data = photo, !working else { return false }
    working = true
    error = nil
    defer { working = false }
    do {
      let source = CGImageSourceCreateWithData(
        data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary)
      let type = source.flatMap { CGImageSourceGetType($0) }.flatMap { UTType($0 as String) }
      let suffix = type?.preferredFilenameExtension ?? "jpg"
      let url = URL.temporaryDirectory.appendingPathComponent("Capture-\(UUID()).\(suffix)")
      defer { try? FileManager.default.removeItem(at: url) }
      try await Task.detached {
        try data.write(
          to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
      }.value
      try await importPhoto(url)
      return true
    } catch {
      self.error = error.localizedDescription
      return false
    }
  }
  nonisolated static func attachLocation(_ coordinate: Coordinate?, to data: Data) throws -> Data {
    guard let coordinate else { return data }
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
      let type = CGImageSourceGetType(source)
    else { throw CameraError.unavailable }
    guard coordinate.isValid else { throw CameraError.unavailable }
    var metadata = (CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]) ?? [:]
    metadata[kCGImagePropertyGPSDictionary] = [
      kCGImagePropertyGPSLatitude: abs(coordinate.latitude),
      kCGImagePropertyGPSLatitudeRef: coordinate.latitude < 0 ? "S" : "N",
      kCGImagePropertyGPSLongitude: abs(coordinate.longitude),
      kCGImagePropertyGPSLongitudeRef: coordinate.longitude < 0 ? "W" : "E",
    ]
    let bytes = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(bytes, type, 1, nil) else {
      throw CameraError.unavailable
    }
    CGImageDestinationAddImageFromSource(destination, source, 0, metadata as CFDictionary)
    guard CGImageDestinationFinalize(destination) else { throw CameraError.unavailable }
    return bytes as Data
  }
}

private struct CameraPreview: UIViewRepresentable {
  let box: CaptureSessionBox
  func makeUIView(context: Context) -> Preview {
    let view = Preview()
    view.layerView.session = box.session
    view.layerView.videoGravity = .resizeAspectFill
    return view
  }
  func updateUIView(_ uiView: Preview, context: Context) {}
  final class Preview: UIView {
    override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    var layerView: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
  }
}

@MainActor struct CameraView: View {
  @Environment(\.dismiss) private var dismiss
  let usePhoto: @MainActor (URL) async throws -> Void
  @State private var controller: CameraController
  init(
    controller: CameraController = CameraController(),
    usePhoto: @escaping @MainActor (URL) async throws -> Void
  ) {
    self.usePhoto = usePhoto
    _controller = State(initialValue: controller)
  }
  var body: some View {
    NavigationStack {
      VStack(spacing: 16) {
        if let bytes = controller.photo, let image = UIImage(data: bytes) {
          Image(uiImage: image).resizable().scaledToFit().accessibilityLabel("Captured photo")
          Text(controller.locationIncluded ? "Location included" : "No location")
          if let error = controller.error { ErrorNotice(message: error) }
          HStack {
            Button("Retake") {
              controller.photo = nil
              controller.error = nil
            }.disabled(controller.working)
            Button("Use photo") { Task { if await controller.save(using: usePhoto) { dismiss() } } }
              .buttonStyle(.borderedProminent).disabled(controller.working)
          }.frame(minHeight: 44)
        } else if let error = controller.error {
          ContentUnavailableView {
            Label("Camera unavailable", systemImage: "camera")
          } description: {
            Text(error)
          } actions: {
            Link("Open Settings", destination: URL(string: UIApplication.openSettingsURLString)!)
            Button("Choose photos instead") { dismiss() }
          }
        } else {
          CameraPreview(box: controller.engine.box).clipShape(RoundedRectangle(cornerRadius: 12))
          Toggle("Add location", isOn: $controller.includeLocation).onChange(
            of: controller.includeLocation
          ) { _, _ in controller.updateLocation() }
          Toggle("Flash", isOn: $controller.flash)
          HStack {
            Button("Switch camera", systemImage: "arrow.triangle.2.circlepath.camera") {
              Task { await controller.flip() }
            }
            Spacer()
            Button("Take photo", systemImage: "camera.shutter.button") {
              Task { await controller.takePhoto() }
            }.disabled(controller.working).accessibilityIdentifier("camera.capture")
          }.frame(minHeight: 64)
        }
        if controller.working {
          ProgressView(controller.photo == nil ? "Taking photo" : "Saving photo")
        }
      }.padding()
        .navigationTitle("Camera").navigationBarTitleDisplayMode(.inline)
        .toolbar { Button("Close") { dismiss() }.disabled(controller.working) }
    }.interactiveDismissDisabled(controller.working)
      .task { await controller.start() }
      .onDisappear {
        Task { await controller.stop() }
      }
  }
}
