import CoreTransferable
import PhotosUI
import PinVoyagerData
import PinVoyagerDomain
import SwiftUI
import UniformTypeIdentifiers

private struct SelectedPhotoFile: Transferable {
  let url: URL
  static var transferRepresentation: some TransferRepresentation {
    FileRepresentation(importedContentType: .image) { received in
      let destination = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        .appendingPathExtension(received.file.pathExtension)
      try FileManager.default.copyItem(at: received.file, to: destination)
      return Self(url: destination)
    }
  }
}

struct PickerPhotoSource: PhotoAcquiring {
  let item: PhotosPickerItem
  let filename: String
  func acquire() async throws -> URL {
    guard let file = try await item.loadTransferable(type: SelectedPhotoFile.self) else {
      throw FileVaultError.missingSource
    }
    return file.url
  }
}

struct PhotoImportView: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let voyage: LocalVoyage
  @State private var selection: [PhotosPickerItem] = []
  @State private var presentation: PhotoImportPresentation?
  private var preparing: Bool { presentation?.preparing == true }
  @State private var publicConsent = false
  @State private var cellularException = false
  @State private var camera = false

  var body: some View {
    NavigationStack {
      List {
        Section {
          Text(voyage.fields.title).font(.headline)
          if voyage.confirmedPublished {
            Toggle("New photos will become public as they upload", isOn: $publicConsent)
            Text("You can make the voyage private from its review screen before adding photos.")
              .font(.footnote)
          }
          if model.workspace?.policy.wifiOnly == true {
            Toggle("Allow cellular for this import only", isOn: $cellularException)
              .disabled(preparing)
          }
          PhotosPicker(
            selection: $selection, maxSelectionCount: 100, selectionBehavior: .ordered,
            matching: .images, preferredItemEncoding: .current
          ) {
            Label("Choose photos", systemImage: "photo.on.rectangle.angled")
          }.disabled(preparing || (voyage.confirmedPublished && !publicConsent))
          Button("Take photo", systemImage: "camera") { camera = true }.disabled(
            preparing || (voyage.confirmedPublished && !publicConsent))
        }
        if preparing {
          Section {
            ProgressView(presentation?.progress ?? "Saving selected photographs")
            Button("Stop importing remaining photos", role: .destructive) {
              Task { await presentation?.stop() }
            }
          }
        }
        ForEach(Array((presentation?.failures ?? []).enumerated()), id: \.offset) { _, failure in
          ErrorNotice(message: failure)
        }
        Section("Saved photos") {
          ForEach(model.workspace?.photos(for: voyage.id) ?? []) { photo in
            HStack {
              LocalPhotoImage(photo: photo).frame(width: 64, height: 64)
              VStack(alignment: .leading) {
                Text(photo.filename)
                Text(photo.transfer.label).font(.caption).foregroundStyle(.secondary)
              }
            }
          }
        }
      }
      .navigationTitle("Add photos")
      .toolbar { Button("Close") { dismiss() }.disabled(preparing) }
      .onChange(of: selection) { _, items in Task { await importSelection(items) } }
      .fullScreenCover(isPresented: $camera) {
        CameraView(controller: model.makeCamera()) { url in try await saveImportedFile(url) }
      }
    }.interactiveDismissDisabled(preparing)
  }

  private func importSelection(_ items: [PhotosPickerItem]) async {
    guard !items.isEmpty, !preparing, let accountId = model.activeAccountId,
      let vault = model.vault
    else { return }
    let presentation =
      self.presentation
      ?? PhotoImportPresentation(
        service: PhotoImportService(store: model.store, vault: vault))
    self.presentation = presentation
    let sources = items.enumerated().map {
      PickerPhotoSource(item: $0.element, filename: "Photo \($0.offset + 1)")
    }
    await presentation.importPhotos(
      sources, accountId: accountId, voyageId: voyage.id,
      maxBytes: model.capabilities?.maxUploadBytes ?? 25 * 1_024 * 1_024,
      publicConsent: publicConsent, cellularException: cellularException
    ) { await model.reloadWorkspace() }
    selection = []
    cellularException = false
    await model.sync()
  }

  private func saveImportedFile(_ url: URL) async throws {
    guard let accountId = model.activeAccountId, let vault = model.vault else {
      throw WorkspaceError.locked
    }
    try await PhotoImportService(store: model.store, vault: vault).saveFile(
      url,
      accountId: accountId, voyageId: voyage.id,
      maxBytes: model.capabilities?.maxUploadBytes ?? 25 * 1_024 * 1_024,
      publicConsent: publicConsent, cellularException: cellularException)
    cellularException = false
    await model.reloadWorkspace()
    model.scheduleSync()
  }
}
