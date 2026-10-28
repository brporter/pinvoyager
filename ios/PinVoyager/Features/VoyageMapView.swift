import MapKit
import PinVoyagerDomain
import SwiftUI

struct VoyageMapView: View {
  @Environment(AppModel.self) private var model
  let pins: [MapPhotoPin]
  var selectedPhotoId: UUID?
  let select: ([UUID]) -> Void
  @State private var position = MapCameraPosition.automatic
  @State private var zoom: Double = 5
  var body: some View {
    if model.offline {
      ContentUnavailableView(
        "Map unavailable offline", systemImage: "map",
        description: Text("Saved photographs and location precision are still available below."))
    } else {
      GeometryReader { geometry in
        Map(position: $position, interactionModes: [.pan, .zoom]) {
          ForEach(MapGeometry.cluster(pins, zoom: zoom)) { cluster in
            if let precision = cluster.coordinate.precisionM {
              MapCircle(center: cluster.coordinate.mapCoordinate, radius: sqrt(2) * precision)
                .foregroundStyle(Design.pin.opacity(0.15))
            }
            Annotation("Photographs", coordinate: cluster.coordinate.mapCoordinate) {
              Button {
                select(cluster.members)
              } label: {
                Label("\(cluster.members.count)", systemImage: "mappin.circle.fill").padding(10)
                  .background(
                    selectedPhotoId.map { cluster.members.contains($0) } == true
                      ? Color.accentColor : Design.pin, in: Capsule()
                  ).foregroundStyle(.white)
              }.accessibilityLabel("View \(cluster.members.count) photographs at this location")
                .frame(minWidth: 44, minHeight: 44)
            }
          }
        }
        .mapStyle(.standard(elevation: .flat))
        .mapControls {
          MapCompass()
          MapScaleView()
        }
        .overlay(alignment: .topTrailing) {
          Button("Fit voyage", systemImage: "arrow.up.left.and.arrow.down.right") { fit() }
            .buttonStyle(.bordered).background(.regularMaterial, in: Capsule()).padding(8)
        }
        .onMapCameraChange(frequency: .onEnd) { context in
          zoom = Double(
            MapGeometry.discoveryZoom(
              width: geometry.size.width, worldFraction: context.region.span.longitudeDelta / 360))
        }
        .onAppear { if selectedPhotoId == nil { fit() } else { focusSelection() } }
        .onChange(of: selectedPhotoId) { focusSelection() }
      }.frame(minHeight: 320)
    }
  }
  private func focusSelection() {
    guard let pin = pins.first(where: { $0.id == selectedPhotoId }) else { return }
    position = .region(
      MKCoordinateRegion(
        center: pin.coordinate.mapCoordinate,
        span: MKCoordinateSpan(latitudeDelta: 0.05, longitudeDelta: 0.05)))
  }
  private func fit() {
    guard let bounds = MapGeometry.bounds(pins.map(\.coordinate)) else { return }
    let half = pins.compactMap(\.coordinate.precisionM).max() ?? 0
    let minimumLatitudeSpan = max(0.01, 4 * sqrt(2) * half / 111_320)
    let minimumLongitudeSpan = minimumLatitudeSpan / max(0.087, cos(bounds.latitude * .pi / 180))
    position = .region(
      MKCoordinateRegion(
        center: CLLocationCoordinate2D(latitude: bounds.latitude, longitude: bounds.longitude),
        span: MKCoordinateSpan(
          latitudeDelta: min(170, max(minimumLatitudeSpan, bounds.latitudeSpan * 1.3)),
          longitudeDelta: min(360, max(minimumLongitudeSpan, bounds.longitudeSpan * 1.3)))))
  }
}
