import Observation
import SwiftUI

@MainActor @Observable final class WindowContext {
  weak var window: UIWindow?
  func anchor() -> UIWindow { window ?? UIWindow() }
}

struct WindowAccessor: UIViewRepresentable {
  let context: WindowContext
  func makeUIView(context: Context) -> WindowProbe { WindowProbe(windowContext: self.context) }
  func updateUIView(_ view: WindowProbe, context: Context) { view.windowContext = self.context }
}

final class WindowProbe: UIView {
  weak var windowContext: WindowContext?
  init(windowContext: WindowContext) {
    self.windowContext = windowContext
    super.init(frame: .zero)
    isUserInteractionEnabled = false
    isAccessibilityElement = false
  }
  required init?(coder: NSCoder) { super.init(coder: coder) }
  override func didMoveToWindow() {
    super.didMoveToWindow()
    windowContext?.window = window
  }
}
