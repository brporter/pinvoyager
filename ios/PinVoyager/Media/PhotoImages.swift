import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

private struct RemoteImageRequest: Hashable {
  let path: String?
  let accountId: UUID?
  let access: PublicAccess
}

struct RemoteImage: View {
  @Environment(AppModel.self) private var model
  let path: String?
  var accountId: UUID?
  @State private var image: UIImage?
  @State private var failed = false
  @State private var loadedAccess: PublicAccess?
  var body: some View {
    Group {
      if let image, loadedAccess == model.publicAccess, !model.offline {
        Image(uiImage: image).resizable().scaledToFill()
      } else {
        PhotoPlaceholder(label: failed ? "Photograph unavailable" : "Photograph")
      }
    }
    .clipped()
    .task(id: RemoteImageRequest(path: path, accountId: accountId, access: model.publicAccess)) {
      let access = model.publicAccess
      image = nil
      failed = false
      guard let path, !model.offline else { return }
      do {
        let bytes = try await model.client.image(path, accountId: accountId)
        guard !Task.isCancelled, access == model.publicAccess, !model.offline else { return }
        loadedAccess = access
        image = UIImage(data: bytes)
        failed = image == nil
      } catch { if access == model.publicAccess, !Task.isCancelled { failed = true } }
    }
  }
}

struct LocalPhotoImage: View {
  @Environment(AppModel.self) private var model
  let photo: LocalPhoto
  var fullSize = false
  var selected = true
  var onZoomChanged: ((Bool) -> Void)?
  @State private var image: UIImage?
  @State private var loadedAccount: UUID?
  var body: some View {
    Group {
      if let image, model.workspace != nil, loadedAccount == model.activeAccountId {
        if fullSize {
          ZoomablePhoto(image: image, resetZoom: !selected, onZoomChanged: onZoomChanged) {
            if photo.source == nil, photo.offlineCopy?.variants["full"] == nil {
              Task { await model.downloadFullCopy(photo.id) }
            }
          }
        } else {
          Image(uiImage: image).resizable().scaledToFit()
        }
      } else {
        PhotoPlaceholder(label: photo.filename)
      }
    }.task(id: imagePath) {
      image = nil
      guard let accountId = model.activeAccountId, let vault = model.vault,
        let path = imagePath
      else { return }
      do {
        let bytes = try await vault.read(path, accountId: accountId)
        guard model.activeAccountId == accountId, model.workspace != nil, !Task.isCancelled else {
          return
        }
        loadedAccount = accountId
        image = UIImage(data: bytes)
      } catch {
        model.error = "This local photo could not be opened. Its saved record has been kept."
      }
    }.accessibilityLabel(photo.filename)
  }
}

extension LocalPhotoImage {
  private var imagePath: String? {
    fullSize
      ? photo.source?.relativePath ?? photo.offlineCopy?.variants["full"]
        ?? photo.offlineCopy?.variants["display"]
      : photo.previewPath ?? photo.offlineCopy?.variants["thumb"]
  }
}

struct ZoomablePhoto: UIViewRepresentable {
  let image: UIImage
  var resetZoom = false
  var onZoomChanged: ((Bool) -> Void)?
  var onZoom: (() -> Void)?
  func makeUIView(context: Context) -> UIScrollView {
    let scroll = PhotoScrollView()
    scroll.minimumZoomScale = 1
    scroll.maximumZoomScale = 5
    scroll.delegate = context.coordinator
    let view = scroll.photo
    view.image = image
    view.contentMode = .scaleAspectFit
    view.accessibilityLabel = "Photograph. Pinch to zoom."
    context.coordinator.imageView = view
    context.coordinator.onZoomChanged = onZoomChanged
    context.coordinator.onZoom = onZoom
    let tap = UITapGestureRecognizer(
      target: context.coordinator, action: #selector(Coordinator.reset(_:)))
    tap.numberOfTapsRequired = 2
    scroll.addGestureRecognizer(tap)
    return scroll
  }
  func updateUIView(_ scroll: UIScrollView, context: Context) {
    context.coordinator.imageView?.image = image
    context.coordinator.onZoomChanged = onZoomChanged
    context.coordinator.onZoom = onZoom
    if resetZoom && scroll.zoomScale != 1 { scroll.setZoomScale(1, animated: false) }
  }
  func makeCoordinator() -> Coordinator { Coordinator() }
  final class Coordinator: NSObject, UIScrollViewDelegate {
    weak var imageView: UIImageView?
    var onZoom: (() -> Void)?
    var onZoomChanged: ((Bool) -> Void)?
    private var zoomed = false
    private var requestedFull = false
    func scrollViewDidZoom(_ scrollView: UIScrollView) {
      let active = scrollView.zoomScale > 1.01
      if active != zoomed {
        zoomed = active
        onZoomChanged?(active)
      }
      guard scrollView.zoomScale > 1.1, !requestedFull else { return }
      requestedFull = true
      onZoom?()
    }
    func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }
    @objc func reset(_ gesture: UITapGestureRecognizer) {
      (gesture.view as? UIScrollView)?.setZoomScale(
        1, animated: !UIAccessibility.isReduceMotionEnabled)
    }
  }
}

private final class PhotoScrollView: UIScrollView {
  let photo = UIImageView()
  override init(frame: CGRect) {
    super.init(frame: frame)
    addSubview(photo)
  }
  required init?(coder: NSCoder) {
    super.init(coder: coder)
    addSubview(photo)
  }
  override func layoutSubviews() {
    super.layoutSubviews()
    if zoomScale == 1 {
      photo.frame = bounds
      contentSize = bounds.size
    }
  }
}
