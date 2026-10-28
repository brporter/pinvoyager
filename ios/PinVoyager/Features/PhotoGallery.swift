import Observation
import PinVoyagerDomain
import SwiftUI

@MainActor @Observable
final class GalleryNavigation {
  private(set) var ids: [UUID]
  private(set) var selectedId: UUID?
  var zoomed = false
  var index: Int { ids.firstIndex { $0 == selectedId } ?? 0 }
  init(ids: [UUID], selectedId: UUID?) {
    self.ids = ids
    self.selectedId = selectedId.flatMap { ids.contains($0) ? $0 : nil } ?? ids.first
  }
  func select(_ id: UUID) {
    guard ids.contains(id), id != selectedId else { return }
    selectedId = id
    zoomed = false
  }
  func move(_ amount: Int) {
    let next = index + amount
    guard ids.indices.contains(next) else { return }
    select(ids[next])
  }
  func refresh(_ next: [UUID]) {
    let previousIndex = index
    ids = next
    guard selectedId.map({ !next.contains($0) }) ?? true else { return }
    selectedId = next.isEmpty ? nil : next[min(previousIndex, next.count - 1)]
    zoomed = false
  }
}

struct PhotoGallery<ImageContent: View, Thumbnail: View, Information: View>: View {
  @Environment(\.dismiss) private var dismiss
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  let ids: [UUID]
  let image: (UUID, Bool, Binding<Bool>) -> ImageContent
  let thumbnail: (UUID) -> Thumbnail
  let information: (UUID) -> Information
  var showOnMap: ((UUID) -> Void)?
  var canShowOnMap: (UUID) -> Bool = { _ in false }
  var report: ((UUID) -> Void)?
  @State private var navigation: GalleryNavigation
  @State private var info = false
  init(
    ids: [UUID], selectedId: UUID?, report: ((UUID) -> Void)? = nil,
    @ViewBuilder image: @escaping (UUID, Bool, Binding<Bool>) -> ImageContent,
    @ViewBuilder thumbnail: @escaping (UUID) -> Thumbnail,
    @ViewBuilder information: @escaping (UUID) -> Information,
    showOnMap: ((UUID) -> Void)? = nil, canShowOnMap: @escaping (UUID) -> Bool = { _ in false }
  ) {
    self.ids = ids
    self.image = image
    self.thumbnail = thumbnail
    self.information = information
    self.showOnMap = showOnMap
    self.canShowOnMap = canShowOnMap
    self.report = report
    _navigation = State(initialValue: GalleryNavigation(ids: ids, selectedId: selectedId))
  }
  var body: some View {
    NavigationStack {
      ScrollView(.horizontal) {
        LazyHStack(spacing: 0) {
          ForEach(ids, id: \.self) { id in
            image(
              id, navigation.selectedId == id,
              Binding(
                get: { navigation.selectedId == id && navigation.zoomed },
                set: { if navigation.selectedId == id { navigation.zoomed = $0 } })
            )
            .containerRelativeFrame([.horizontal, .vertical])
            .id(id)
          }
        }.scrollTargetLayout()
      }
      .scrollTargetBehavior(.paging)
      .scrollPosition(
        id: Binding(
          get: { navigation.selectedId }, set: { if let id = $0 { navigation.select(id) } })
      )
      .scrollIndicators(.hidden)
      .scrollDisabled(navigation.zoomed)
      .background(.black)
      .safeAreaInset(edge: .bottom) { thumbnails }
      .navigationTitle(ids.isEmpty ? "No photos" : "\(navigation.index + 1) of \(ids.count)")
      .toolbar {
        ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
        ToolbarItem(placement: .primaryAction) {
          Button("Info", systemImage: "info.circle") { info = true }.disabled(ids.isEmpty)
        }
        ToolbarItemGroup(placement: .bottomBar) {
          Button("Previous", systemImage: "chevron.left") { navigation.move(-1) }.disabled(
            navigation.index == 0)
          Spacer()
          Button("Next", systemImage: "chevron.right") { navigation.move(1) }.disabled(
            navigation.index >= ids.count - 1)
        }
      }
      .sheet(isPresented: $info) {
        if let id = navigation.selectedId {
          NavigationStack {
            Form {
              information(id)
              if canShowOnMap(id), let showOnMap {
                Button("Show on map") {
                  info = false
                  showOnMap(id)
                  dismiss()
                }
              }
              if let report {
                Button("Report photo") {
                  info = false
                  report(id)
                  dismiss()
                }
              }
            }.navigationTitle("Photo information").toolbar { Button("Done") { info = false } }
          }.presentationDetents([.medium, .large])
        }
      }
    }.onChange(of: ids) { _, next in navigation.refresh(next) }
  }
  private var thumbnails: some View {
    ScrollViewReader { proxy in
      ScrollView(.horizontal) {
        LazyHStack(spacing: 8) {
          ForEach(Array(ids.enumerated()), id: \.element) { index, id in
            Button {
              navigation.select(id)
            } label: {
              thumbnail(id).frame(width: 56, height: 56).clipShape(
                RoundedRectangle(cornerRadius: 6)
              )
              .overlay(
                RoundedRectangle(cornerRadius: 6).stroke(
                  navigation.selectedId == id ? Color.accentColor : Color.clear, lineWidth: 3))
            }.id(id).buttonStyle(.plain).accessibilityLabel("Thumbnail \(index + 1)")
              .accessibilityAddTraits(navigation.selectedId == id ? .isSelected : [])
          }
        }.padding(8)
      }.frame(height: 72).background(.ultraThinMaterial)
        .onChange(of: navigation.selectedId, initial: true) { _, id in
          if let id {
            withAnimation(reduceMotion ? nil : .default) { proxy.scrollTo(id, anchor: .center) }
          }
        }
    }
  }
}

struct OwnerGallery: View {
  @Environment(AppModel.self) private var model
  let voyageId: UUID
  let selectedId: UUID
  var members: Set<UUID>?
  var showOnMap: ((UUID) -> Void)?
  private var photos: [LocalPhoto] {
    (model.workspace?.photos(for: voyageId) ?? []).filter { members?.contains($0.id) != false }
  }
  var body: some View {
    PhotoGallery(ids: photos.map(\.id), selectedId: selectedId) { id, selected, zoomed in
      if let photo = photos.first(where: { $0.id == id }) {
        VStack {
          if let remote = photo.serverId, photo.source == nil, photo.offlineCopy == nil {
            RemoteOwnerGalleryImage(
              photo: photo, remoteId: remote, selected: selected, zoomed: zoomed)
          } else {
            LocalPhotoImage(
              photo: photo, fullSize: true, selected: selected,
              onZoomChanged: { zoomed.wrappedValue = $0 })
          }
          Text(
            photo.source != nil
              ? "Original saved on this iPhone"
              : photo.offlineCopy != nil ? "Downloaded owner copy" : "Website photo"
          ).font(.caption)
          if let coordinate = photo.coordinate {
            Text(
              "Private recorded location: \(coordinate.latitude.formatted()), \(coordinate.longitude.formatted())"
            ).font(.caption)
          }
        }
      }
    } thumbnail: { id in
      if let photo = photos.first(where: { $0.id == id }) {
        if let remote = photo.serverId, photo.source == nil, photo.offlineCopy == nil {
          RemoteImage(path: "/api/photos/\(remote)/thumb", accountId: model.activeAccountId)
        } else {
          LocalPhotoImage(photo: photo)
        }
      }
    } information: { id in
      if let photo = photos.first(where: { $0.id == id }) {
        LabeledContent("Filename", value: photo.filename)
        LabeledContent("Dimensions", value: "\(photo.width) × \(photo.height)")
        LabeledContent("Upload state", value: photo.transfer.label)
        if let date = photo.capturedAt { LabeledContent("Captured", value: date) }
        if let coordinate = photo.coordinate {
          LabeledContent(
            "Exact location — private metadata",
            value: "\(coordinate.latitude.formatted()), \(coordinate.longitude.formatted())")
        } else {
          Text("No recorded location")
        }
      }
    } showOnMap: {
      showOnMap?($0)
    } canShowOnMap: { id in
      photos.first { $0.id == id }?.coordinate != nil
    }
  }
}

private struct RemoteOwnerGalleryImage: View {
  let photo: LocalPhoto
  let remoteId: UUID
  let selected: Bool
  let zoomed: Binding<Bool>
  var body: some View {
    GalleryImage(
      displayPath: "/api/photos/\(remoteId)/display", fullPath: "/api/photos/\(remoteId)/full",
      selected: selected, onZoomChanged: { zoomed.wrappedValue = $0 })
  }
}
