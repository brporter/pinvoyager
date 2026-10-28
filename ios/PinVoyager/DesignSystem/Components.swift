import PinVoyagerDomain
import SwiftUI

enum Design {
  static let spacing: CGFloat = 16
  static let pin = Color("MapPin")
}

struct ErrorNotice: View {
  let message: String
  var retryTitle = "Try again"
  var retry: (() -> Void)?
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Label(message, systemImage: "exclamationmark.circle").foregroundStyle(.secondary)
      if let retry { Button(retryTitle, action: retry).frame(minHeight: 44) }
    }.padding().accessibilityElement(children: .contain)
  }
}

struct VoyageHeading: View {
  let title: String
  let description: String?
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text(title).font(.title2.bold()).textSelection(.enabled)
      if let description, !description.isEmpty {
        Text(description).foregroundStyle(.secondary).textSelection(.enabled)
      }
    }.frame(maxWidth: .infinity, alignment: .leading)
  }
}

struct VisibilityLabel: View {
  let published: Bool
  var body: some View {
    Label(published ? "Public" : "Draft", systemImage: published ? "globe" : "lock").font(
      .subheadline)
  }
}

struct PhotoPlaceholder: View {
  var label = "Photograph"
  var body: some View {
    Rectangle().fill(.quaternary).overlay { Image(systemName: "photo").foregroundStyle(.secondary) }
      .accessibilityLabel(label)
  }
}

extension AppTab {
  var title: String {
    switch self {
    case .discover: "Discover"
    case .voyages: "My Voyages"
    case .sync: "Sync"
    case .account: "Account"
    }
  }
  var symbol: String {
    switch self {
    case .discover: "map"
    case .voyages: "suitcase"
    case .sync: "arrow.triangle.2.circlepath"
    case .account: "person.crop.circle"
    }
  }
}

extension TransferState {
  var label: String {
    switch self {
    case .acquiring: "Saving on this iPhone"
    case .needsSource: "Choose this photo again"
    case .saved, .waiting: "Saved on this iPhone"
    case .scheduled: "Waiting for iOS"
    case .uploading: "Uploading"
    case .checkingReceipt: "Checking upload receipt"
    case .accepted: "Uploaded"
    case .needsAttention: "Needs attention"
    case .removing: "Removal waiting to sync"
    case .removed: "Removed"
    }
  }
}
