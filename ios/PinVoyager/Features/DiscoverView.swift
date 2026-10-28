import MapKit
import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct DiscoverView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @State private var mode = "Recent"
  @State private var hasShownMap = false
  var body: some View {
    VStack(spacing: 0) {
      Picker("Discover view", selection: $mode) {
        Text("Recent").tag("Recent")
        Text("Map").tag("Map")
      }.pickerStyle(.segmented).padding()
      ZStack {
        List {
          if model.discover.isEmpty && !model.loadingDiscover {
            ContentUnavailableView(
              "\(model.offline ? "Not available offline" : "No public voyages yet")",
              systemImage: "map",
              description: Text(
                model.offline
                  ? "Your saved voyages are in My Voyages."
                  : "Published travelogues will appear here."))
          }
          ForEach(model.discover) { voyage in VoyageCard(voyage: voyage) }
          if let error = model.discoverError {
            ErrorNotice(message: error) { Task { await model.loadDiscover(refresh: true) } }
          }
          if model.loadingDiscover { ProgressView("Loading voyages") }
          if model.discoverCursor != nil {
            Button("Load more") { Task { await model.loadDiscover(refresh: false) } }.disabled(
              model.loadingDiscover || model.offline)
          }
        }.listStyle(.plain).refreshable { await model.loadDiscover(refresh: true) }
          .opacity(mode == "Recent" ? 1 : 0)
          .allowsHitTesting(mode == "Recent")
          .accessibilityHidden(mode != "Recent")
        if hasShownMap {
          DiscoveryMapView(active: mode == "Map")
            .opacity(mode == "Map" ? 1 : 0)
            .allowsHitTesting(mode == "Map")
            .accessibilityHidden(mode != "Map")
        }
      }
    }
    .onChange(of: mode) { _, mode in
      if mode == "Map" { hasShownMap = true }
    }
    .navigationTitle("Discover")
    .toolbar {
      Button("Create a voyage", systemImage: "plus") {
        router.requestCreation(hasWorkspace: model.workspace != nil)
      }
    }
    .task {
      if model.discover.isEmpty && !AppBootstrap.isUITesting {
        await model.loadDiscover(refresh: true)
      }
    }
  }
}

struct VoyageCard: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  let voyage: VoyageSummary
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      NavigationLink(value: SceneDestination.voyage(voyage.id)) {
        VStack(alignment: .leading, spacing: 12) {
          RemoteImage(path: voyage.coverUrl).frame(height: 200).clipShape(
            RoundedRectangle(cornerRadius: 12))
          Text(voyage.title).font(.title3.bold()).foregroundStyle(.primary)
          if let description = voyage.description {
            Text(description).lineLimit(3).foregroundStyle(.secondary)
          }
        }
      }.buttonStyle(.plain)
      Button {
        router.discoverPath.append(.profile(voyage.ownerId.uuidString))
      } label: {
        Label("View traveler", systemImage: "person.crop.circle")
      }.buttonStyle(.borderless).font(.subheadline).frame(minHeight: 44)
    }.padding(.vertical, 8)
  }
}

struct DiscoveryMapView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  var active = true
  @State private var position = MapCameraPosition.region(
    MKCoordinateRegion(
      center: CLLocationCoordinate2D(latitude: 25, longitude: 0),
      span: MKCoordinateSpan(latitudeDelta: 100, longitudeDelta: 180)))
  @State private var region = MKCoordinateRegion(
    center: CLLocationCoordinate2D(latitude: 25, longitude: 0),
    span: MKCoordinateSpan(latitudeDelta: 100, longitudeDelta: 180))
  @State private var loadedAccess: PublicAccess?
  @State private var clusters: [DiscoveryCluster] = []
  @State private var truncated = false
  @State private var error: String?
  @State private var selectedCluster: DiscoveryCluster?
  @State private var requestKey = "initial"
  var body: some View {
    GeometryReader { geometry in
      VStack(spacing: 0) {
        if model.publicAccess.pendingBlocks {
          ContentUnavailableView(
            "Map waiting for blocked-account changes",
            systemImage: "person.crop.circle.badge.clock",
            description: Text("Open Sync to finish blocking before reloading the map."))
        } else if model.offline {
          ContentUnavailableView("Map unavailable offline", systemImage: "wifi.slash")
        } else {
          Map(position: $position, interactionModes: [.pan, .zoom]) {
            ForEach(loadedAccess == model.publicAccess ? clusters : []) { cluster in
              Annotation(
                "\(cluster.count) photographs", coordinate: cluster.coordinate.mapCoordinate
              ) {
                Button {
                  activate(cluster, width: geometry.size.width)
                } label: {
                  Label("\(cluster.count)", systemImage: "mappin.circle.fill").padding(8)
                    .background(Design.pin, in: Capsule()).foregroundStyle(.white)
                }.accessibilityLabel(
                  "\(cluster.count) photographs in \(cluster.voyages.count) voyages")
              }
            }
          }.mapStyle(.standard(elevation: .flat)).mapControls {
            MapCompass()
            MapScaleView()
          }
          .onMapCameraChange(frequency: .onEnd) { context in
            region = context.region
            requestKey =
              "\(context.region.center.latitude),\(context.region.center.longitude),\(context.region.span.longitudeDelta)"
          }
          if truncated { Text("Zoom in to see more photographs").font(.footnote).padding(8) }
          if let error { ErrorNotice(message: error) { requestKey = UUID().uuidString } }
          List(visibleVoyages) { voyage in
            NavigationLink(voyage.title, value: SceneDestination.voyage(voyage.id))
          }
          .frame(maxHeight: 180).accessibilityLabel("Visible voyages")
        }
      }
      .task(id: DiscoveryRequest(region: requestKey, access: model.publicAccess, active: active)) {
        selectedCluster = nil
        guard active else { return }
        await query(width: geometry.size.width)
      }
    }
    .sheet(item: $selectedCluster) { cluster in
      NavigationStack {
        List(cluster.voyages) { voyage in
          Button(voyage.title) {
            selectedCluster = nil
            router.discoverPath.append(.voyage(voyage.id))
          }.accessibilityIdentifier("discovery.cluster-voyage.\(voyage.id)")
        }
        .navigationTitle("Voyages at this location").toolbar {
          Button("Close") { selectedCluster = nil }
        }
      }.presentationDetents([.medium, .large])
    }
  }
  private var visibleVoyages: [ClusterVoyage] {
    guard loadedAccess == model.publicAccess else { return [] }
    var seen: Set<UUID> = []
    return clusters.flatMap(\.voyages).filter { seen.insert($0.id).inserted }
  }
  private func query(width: Double) async {
    let access = model.publicAccess
    guard !model.offline, !access.pendingBlocks else {
      clusters = []
      return
    }
    do {
      try await Task.sleep(for: .milliseconds(300))
      let fraction = min(1, region.span.longitudeDelta / 360)
      let fullWorld = region.span.longitudeDelta >= 360
      let values: [(String, Double)] = [
        (
          "west",
          fullWorld
            ? -180
            : MapGeometry.wrapLongitude(region.center.longitude - region.span.longitudeDelta / 2)
        ),
        (
          "east",
          fullWorld
            ? 180
            : MapGeometry.wrapLongitude(region.center.longitude + region.span.longitudeDelta / 2)
        ),
        ("south", max(-90, region.center.latitude - region.span.latitudeDelta / 2)),
        ("north", min(90, region.center.latitude + region.span.latitudeDelta / 2)),
        ("zoom", Double(MapGeometry.discoveryZoom(width: width, worldFraction: fraction))),
      ]
      let response: DiscoveryResponse = try await model.client.get(
        path: "/api/map", query: values.map { URLQueryItem(name: $0.0, value: String($0.1)) },
        accountId: model.signedIn ? model.activeAccountId : nil)
      guard !Task.isCancelled, access == model.publicAccess, !model.offline else { return }
      loadedAccess = access
      clusters = response.clusters
      truncated = response.truncated
      error = nil
    } catch is CancellationError {} catch {
      if !Task.isCancelled, access == model.publicAccess { self.error = error.localizedDescription }
    }
  }
  private func activate(_ cluster: DiscoveryCluster, width: Double) {
    if cluster.count == 1 {
      router.discoverPath.append(.voyage(cluster.voyageId))
    } else if MapGeometry.discoveryZoom(
      width: width, worldFraction: region.span.longitudeDelta / 360) >= 20
    {
      selectedCluster = cluster
    } else {
      position = .region(
        MKCoordinateRegion(
          center: cluster.coordinate.mapCoordinate,
          span: MKCoordinateSpan(
            latitudeDelta: region.span.latitudeDelta / 4,
            longitudeDelta: region.span.longitudeDelta / 4)))
    }
  }
}

extension Coordinate {
  var mapCoordinate: CLLocationCoordinate2D {
    CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
  }
}

private struct DiscoveryRequest: Hashable {
  let region: String
  let access: PublicAccess
  let active: Bool
}
