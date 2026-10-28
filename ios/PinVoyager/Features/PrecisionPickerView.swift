import MapKit
import PinVoyagerDomain
import SwiftUI

struct PrecisionPickerView: View {
  @Environment(\.dismiss) private var dismiss
  @State private var choice: Granularity
  let coordinate: Coordinate?
  let confirm: (Granularity) -> Void
  init(initial: Granularity, coordinate: Coordinate?, confirm: @escaping (Granularity) -> Void) {
    _choice = State(initialValue: initial)
    self.coordinate = coordinate
    self.confirm = confirm
  }
  var body: some View {
    NavigationStack {
      List {
        Section {
          ForEach(Granularity.allCases, id: \.self) { value in
            Button {
              choice = value
            } label: {
              HStack {
                Text(value.label)
                Spacer()
                if value == choice { Image(systemName: "checkmark").accessibilityLabel("Selected") }
              }
            }.accessibilityLabel(value.label).accessibilityAddTraits(
              value == choice ? .isSelected : [])
          }
        }
        Section("Public location example") {
          PrecisionPreview(granularity: choice, coordinate: coordinate)
        }
        Section {
          Text(
            "This controls locations shown publicly. Original photo locations remain available to you. Photos can reveal places through their content too."
          )
          if choice == .exact {
            Label("Exact recorded locations will be public", systemImage: "location.fill")
              .foregroundStyle(.orange)
          }
        }
      }.navigationTitle("Location precision").toolbar {
        ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
        ToolbarItem(placement: .confirmationAction) {
          Button("Done") {
            confirm(choice)
            dismiss()
          }.disabled(!choice.canPublish)
        }
      }
    }
  }
}

struct PrecisionPreview: View {
  @Environment(AppModel.self) private var model
  let granularity: Granularity
  var coordinate: Coordinate?
  private var example: Coordinate {
    granularity.apply(coordinate ?? Coordinate(latitude: 37.7749, longitude: -122.4194))
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Text(coordinate == nil ? "Example location" : "Example using a saved photo").font(.caption)
      if model.offline {
        Image(systemName: granularity == .exact ? "mappin.circle" : "circle.dashed")
          .resizable().scaledToFit().frame(maxWidth: .infinity).frame(height: 72)
          .accessibilityHidden(true)
        Text("Map unavailable offline — schematic example").font(.footnote)
      } else {
        let halfCell = granularity.halfCellM ?? 250
        let latitudeSpan = max(0.02, halfCell * 6 / 111_320)
        Map(
          initialPosition: .region(
            MKCoordinateRegion(
              center: example.mapCoordinate,
              span: MKCoordinateSpan(
                latitudeDelta: latitudeSpan,
                longitudeDelta: latitudeSpan / max(0.087, cos(example.latitude * .pi / 180))))),
          interactionModes: []
        ) {
          if let halfCell = granularity.halfCellM {
            MapCircle(center: example.mapCoordinate, radius: sqrt(2) * halfCell).foregroundStyle(
              Design.pin.opacity(0.25))
          } else {
            Marker("Example exact location", coordinate: example.mapCoordinate).tint(Design.pin)
          }
        }.mapStyle(.standard(elevation: .flat)).id(granularity).frame(height: 180)
      }
      Text(granularity == .exact ? "Example exact location" : "Example approximate area")
      if let halfCell = granularity.halfCellM {
        Text(
          "Approximation radius: \((sqrt(2) * halfCell / 1_000).formatted(.number.precision(.fractionLength(1)))) km"
        )
        .font(.caption)
      }
    }
  }
}
