import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct RemoteVoyageView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @Environment(\.scenePhase) private var phase
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize
  let id: UUID
  @State private var voyage: RemoteVoyage?
  @State private var loadedAccess: PublicAccess?
  @State private var error: String?
  @State private var mode = "Photos"
  @State private var gallery: GallerySelection?
  @State private var reporting = false
  @State private var reportingPhoto: UUID?
  @State private var reportAfterGallery = false
  @State private var selectedMapPhoto: UUID?
  @State private var galleryAnchor: UUID?
  @AccessibilityFocusState private var focusedPhoto: UUID?
  var body: some View {
    Group {
      if model.offline {
        ContentUnavailableView(
          "Not available offline", systemImage: "wifi.slash",
          description: Text(
            "Public voyages require current authorization. Your saved owner work is in My Voyages.")
        )
      } else if let voyage, loadedAccess == model.publicAccess,
        model.workspace?.blockedAccounts.contains(voyage.ownerId) == true
      {
        ContentUnavailableView("Traveler blocked", systemImage: "person.crop.circle.badge.xmark")
      } else if let voyage, loadedAccess == model.publicAccess {
        ScrollView {
          VStack(alignment: .leading, spacing: 16) {
            VoyageHeading(title: voyage.title, description: voyage.description)
            NavigationLink(
              "View traveler", value: SceneDestination.profile(voyage.ownerId.uuidString))
            Text("\(voyage.photoCount) photos · \(voyage.placeCount) places").font(.subheadline)
              .foregroundStyle(.secondary)
            VisibilityLabel(published: voyage.published)
            Picker("Voyage view", selection: $mode) {
              Text("Map").tag("Map")
              Text("Photos").tag("Photos")
            }.pickerStyle(.segmented)
            if mode == "Map" {
              VoyageMapView(
                pins: voyage.photos.compactMap { photo in
                  photo.coordinate.map { MapPhotoPin(id: photo.id, coordinate: $0) }
                }, selectedPhotoId: selectedMapPhoto
              ) { members in
                let photos = voyage.photos.filter { members.contains($0.id) }
                galleryAnchor = photos.first?.id
                gallery = GallerySelection(
                  photos: photos, selectedId: photos.first?.id, members: Set(members))
              }
              Text("\(voyage.unplacedCount) without location").font(.subheadline)
            }
            LazyVGrid(
              columns: dynamicTypeSize.isAccessibilitySize
                ? [GridItem(.flexible())] : [GridItem(.adaptive(minimum: 140), spacing: 8)],
              spacing: 8
            ) {
              ForEach(voyage.photos) { photo in
                Button {
                  galleryAnchor = photo.id
                  gallery = GallerySelection(photos: voyage.photos, selectedId: photo.id)
                } label: {
                  VStack {
                    if photo.variantStatus == .ready {
                      RemoteImage(path: photo.thumbnailUrl, accountId: model.activeAccountId).frame(
                        height: 150)
                    } else {
                      PhotoPlaceholder(
                        label: photo.variantStatus == .failed
                          ? "Could not process" : "Processing photo"
                      ).frame(height: 150)
                    }
                    if photo.variantStatus != .ready {
                      Text(photo.variantStatus == .failed ? "Could not process" : "Processing")
                        .font(.caption)
                    }
                  }
                }.buttonStyle(.plain).clipShape(RoundedRectangle(cornerRadius: 12))
                  .accessibilityFocused($focusedPhoto, equals: photo.id)
                  .accessibilityLabel(
                    "Photo \(photo.position + 1)\(photo.placed ? "" : ", no recorded location")")
              }
            }
            if voyage.photos.isEmpty { ContentUnavailableView("No photos", systemImage: "photo") }
          }.padding()
        }
        .refreshable { await load() }
        .toolbar {
          if voyage.published {
            ShareLink(
              "Share voyage", item: model.webOrigin.url(path: ["voyages", voyage.id.uuidString]))
          }
          Menu("More", systemImage: "ellipsis") {
            Link(
              "Open on website",
              destination: model.webOrigin.url(path: ["voyages", voyage.id.uuidString]))
            if voyage.ownerId != model.activeAccountId {
              Button("Report voyage") {
                reportingPhoto = nil
                reporting = true
              }
              Button("Block traveler", role: .destructive) { block(voyage.ownerId) }
            }
          }
        }
      } else if let error {
        ErrorNotice(message: error) { Task { await load() } }
      } else {
        ProgressView("Loading voyage")
      }
    }
    .navigationTitle(loadedAccess == model.publicAccess ? voyage?.title ?? "Voyage" : "Voyage")
    .navigationBarTitleDisplayMode(.inline)
    .task(id: RemoteRefreshRequest(access: model.publicAccess, active: phase == .active)) {
      voyage = nil
      gallery = nil
      error = nil
      guard phase == .active else { return }
      try? await ForegroundPolling.observe { () -> [String]? in
        guard !model.offline, phase == .active else { return nil }
        await load()
        guard let photos = voyage?.photos, photos.contains(where: { $0.variantStatus == .pending })
        else { return nil }
        return photos.map { "\($0.id):\($0.variantStatus.rawValue)" }
      }
    }
    .fullScreenCover(
      item: $gallery,
      onDismiss: galleryDismissed
    ) { selection in
      let report: ((UUID) -> Void)? =
        voyage?.ownerId != model.activeAccountId
        ? { id in
          reportingPhoto = id
          reportAfterGallery = true
        } : nil
      RemoteGallery(
        selection: selection,
        showOnMap: { id in
          mode = "Map"
          selectedMapPhoto = id
        }, report: report
      )
    }
    .sheet(isPresented: $reporting, onDismiss: router.sheetDismissed) {
      ReportView(
        target: reportingPhoto.map { "photo:\($0)" } ?? "voyage:\(id)",
        subject: reportingPhoto == nil
          ? voyage?.title : "Photograph in \(voyage?.title ?? "this voyage")")
    }
  }
  private func galleryDismissed() {
    focusedPhoto = galleryAnchor
    if reportAfterGallery {
      reportAfterGallery = false
      reporting = true
    }
  }
  private func load() async {
    let access = model.publicAccess
    guard !access.offline else { return }
    do {
      let result = try await model.client.voyage(
        id, accountId: model.signedIn ? model.activeAccountId : nil)
      guard access == model.publicAccess, !Task.isCancelled else { return }
      loadedAccess = access
      if voyage == nil { mode = result.placedCount > 0 ? "Map" : "Photos" }
      voyage = result
      if let open = gallery {
        gallery = GallerySelection(
          photos: result.photos.filter { open.members?.contains($0.id) != false },
          selectedId: open.selectedId, members: open.members, id: open.id)
      }
      error = nil
    } catch {
      guard access == model.publicAccess, !Task.isCancelled else { return }
      voyage = nil
      self.error = error.localizedDescription
    }
  }
  private func block(_ userId: UUID) {
    guard let accountId = model.activeAccountId else {
      router.showingSignIn = true
      return
    }
    Task {
      await model.perform {
        try await model.repository.setBlocked(accountId: accountId, userId: userId, blocked: true)
      }
    }
  }
}

struct ProfileView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  let identifier: String
  @State private var presentation: ProfilePresentation?
  @State private var reporting = false
  var body: some View {
    List {
      if let profile = presentation?.profile, presentation?.loadedAccess == model.publicAccess {
        if model.workspace?.blockedAccounts.contains(profile.user.id) == true {
          Text("Traveler blocked")
        } else {
          Section {
            Text(profile.user.displayName ?? "Traveler").font(.largeTitle.bold())
            Text("Public profile").foregroundStyle(.secondary)
            ShareLink(
              "Share profile",
              item: model.webOrigin.url(path: [
                "u", profile.user.displayName ?? profile.user.id.uuidString,
              ]))
            if profile.user.id == model.activeAccountId {
              Button("My Voyages") { router.selectedTab = .voyages }
            }
          }
          if presentation?.voyages.isEmpty == true {
            ContentUnavailableView("No public voyages", systemImage: "map")
          }
          ForEach(presentation?.voyages ?? []) { VoyageCard(voyage: $0) }
          if presentation?.cursor != nil {
            Button("Load more") { Task { await load(refresh: false) } }.disabled(
              presentation?.loading == true)
          }
        }
      }
      if presentation?.loading == true { ProgressView("Loading public profile") }
      if let error = presentation?.error {
        ErrorNotice(
          message: error,
          retryTitle: presentation?.retryingMore == true ? "Retry loading more" : "Try again"
        ) {
          Task {
            await presentation?.retry(access: model.publicAccess) { $0 == model.publicAccess }
          }
        }
      }
    }.navigationTitle("Traveler").navigationBarTitleDisplayMode(.inline)
      .task(id: model.publicAccess) {
        let presentation =
          self.presentation ?? ProfilePresentation(identifier: identifier, client: model.client)
        self.presentation = presentation
        presentation.invalidate()
        await load(refresh: true)
      }
      .refreshable { await load(refresh: true) }
      .toolbar {
        if let profile = presentation?.profile, profile.user.id != model.activeAccountId {
          Menu("More", systemImage: "ellipsis") {
            Button("Report traveler") { reporting = true }
            Button("Block traveler", role: .destructive) {
              guard let account = model.activeAccountId else {
                router.showingSignIn = true
                return
              }
              Task {
                await model.perform {
                  try await model.repository.setBlocked(
                    accountId: account, userId: profile.user.id, blocked: true)
                }
              }
            }
          }
        }
      }
      .sheet(isPresented: $reporting, onDismiss: router.sheetDismissed) {
        if let profile = presentation?.profile {
          ReportView(
            target: "user:\(profile.user.id)", subject: profile.user.displayName ?? "Traveler")
        }
      }
  }
  private func load(refresh: Bool) async {
    await presentation?.load(refresh: refresh, access: model.publicAccess) {
      $0 == model.publicAccess
    }
  }
}

struct GallerySelection: Identifiable {
  let id: UUID
  let photos: [RemotePhoto]
  let selectedId: UUID?
  var members: Set<UUID>?
  init(photos: [RemotePhoto], selectedId: UUID?, members: Set<UUID>? = nil, id: UUID = UUID()) {
    self.id = id
    self.photos = photos
    self.selectedId = selectedId
    self.members = members
  }
}

struct RemoteGallery: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let selection: GallerySelection
  var showOnMap: ((UUID) -> Void)?
  var report: ((UUID) -> Void)?
  var body: some View {
    PhotoGallery(ids: selection.photos.map(\.id), selectedId: selection.selectedId, report: report)
    { id, selected, zoomed in
      if let photo = selection.photos.first(where: { $0.id == id }) {
        GalleryImage(
          displayPath: photo.displayUrl, fullPath: photo.fullUrl,
          selected: selected, onZoomChanged: { zoomed.wrappedValue = $0 })
      }
    } thumbnail: { id in
      if let photo = selection.photos.first(where: { $0.id == id }) {
        RemoteImage(path: photo.thumbnailUrl, accountId: model.publicAccess.accountId)
      }
    } information: { id in
      if let photo = selection.photos.first(where: { $0.id == id }) {
        LabeledContent("Dimensions", value: "\(photo.width) × \(photo.height)")
        if let date = photo.capturedAt { LabeledContent("Captured", value: date) }
        if let coordinate = photo.coordinate {
          LabeledContent(
            coordinate.precisionM == nil ? "Exact location" : "Approximate location",
            value: "\(coordinate.latitude.formatted()), \(coordinate.longitude.formatted())")
        } else {
          Text("No recorded location")
        }
      }
    } showOnMap: {
      showOnMap?($0)
    } canShowOnMap: { id in
      selection.photos.first { $0.id == id }?.coordinate != nil
    }
    .onChange(of: model.publicAccess) { dismiss() }
  }
}

struct GalleryImage: View {
  @Environment(AppModel.self) private var model
  let displayPath: String
  let fullPath: String
  var selected = true
  var onZoomChanged: ((Bool) -> Void)?
  @State private var image: UIImage?
  @State private var fullResolution = false
  @State private var loadedAccess: PublicAccess?
  @State private var error: String?
  var body: some View {
    Group {
      if let image, loadedAccess == model.publicAccess, !model.offline {
        ZoomablePhoto(image: image, resetZoom: !selected, onZoomChanged: onZoomChanged) {
          fullResolution = true
        }
      } else if let error {
        ErrorNotice(message: error)
      } else {
        ProgressView("Loading photograph")
      }
    }.task(id: GalleryImageRequest(access: model.publicAccess, fullResolution: fullResolution)) {
      let access = model.publicAccess
      if loadedAccess != access { image = nil }
      error = nil
      guard !model.offline else {
        error = "Not available offline"
        return
      }
      do {
        let bytes = try await model.client.image(
          fullResolution ? fullPath : displayPath, accountId: access.accountId)
        guard access == model.publicAccess, !Task.isCancelled else { return }
        loadedAccess = access
        image = UIImage(data: bytes)
        if image == nil { error = "This photograph could not be decoded." }
      } catch {
        if access == model.publicAccess, !Task.isCancelled {
          self.error = error.localizedDescription
        }
      }
    }
  }
}

private struct GalleryImageRequest: Hashable {
  let access: PublicAccess
  let fullResolution: Bool
}

private struct RemoteRefreshRequest: Hashable {
  let access: PublicAccess
  let active: Bool
}
