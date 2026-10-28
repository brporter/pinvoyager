import PhotosUI
import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct PhotoRecoveryView: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let photo: LocalPhoto
  @State private var selected: PhotosPickerItem?
  @State private var maximum = 2048
  @State private var publicConsent = false
  @State private var working = false
  @State private var error: String?
  private var published: Bool {
    model.workspace?.voyages.first { $0.id == photo.voyageId }?.confirmedPublished == true
  }
  var body: some View {
    NavigationStack {
      Form {
        Section {
          Text(photo.filename).font(.headline)
          Text(
            "A replacement gets a new upload identity. Your current photo stays in the voyage until the replacement is saved. Imported originals remain available in your local export."
          )
          if published {
            Toggle("The replacement will become public as it uploads", isOn: $publicConsent)
          }
          PhotosPicker(selection: $selected, matching: .images, preferredItemEncoding: .current) {
            Label("Choose a replacement source", systemImage: "photo")
          }.disabled(working || (published && !publicConsent))
        }
        if photo.source != nil {
          Section("Smaller supported copy") {
            Text(
              "Create a JPEG with fewer pixels. Location and capture date are retained when recorded. The original bytes are kept."
            )
            Picker("JPEG longest edge", selection: $maximum) {
              ForEach([512, 1024, 2048], id: \.self) { Text("\($0) pixels").tag($0) }
            }
            Button("Create smaller JPEG copy") { Task { await recover(smaller: true) } }
              .disabled(working || (published && !publicConsent))
          }
        }
        if working { ProgressView("Saving replacement") }
        if let error { ErrorNotice(message: error) }
      }.navigationTitle("Recover photograph")
        .toolbar { Button("Close") { dismiss() }.disabled(working) }
        .onChange(of: selected) { _, item in
          if item != nil { Task { await recover(smaller: false) } }
        }
    }.interactiveDismissDisabled(working)
  }
  private func recover(smaller: Bool) async {
    guard !working, let accountId = model.activeAccountId, let vault = model.vault else { return }
    working = true
    error = nil
    defer {
      working = false
      selected = nil
    }
    let importer = PhotoImportService(store: model.store, vault: vault)
    let limit = model.capabilities?.maxUploadBytes ?? 25 * 1_024 * 1_024
    do {
      if smaller {
        try await importer.makeSmallerCopy(
          accountId: accountId, photoId: photo.id, maxBytes: limit,
          maxDimension: maximum, publicConsent: publicConsent)
      } else if let selected {
        try await importer.replace(
          PickerPhotoSource(item: selected, filename: photo.filename),
          accountId: accountId, photoId: photo.id, maxBytes: limit, publicConsent: publicConsent)
      } else {
        return
      }
      await model.reloadWorkspace()
      model.scheduleSync()
      dismiss()
    } catch { self.error = error.localizedDescription }
  }
}
