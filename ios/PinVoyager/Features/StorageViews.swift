import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct StorageSettingsSections: View {
  @Environment(AppModel.self) private var model
  @State private var usage = StorageBreakdown()
  @State private var confirmCleanup = false
  @State private var changePreference = false
  private var keepOriginals: Bool { (model.workspace?.policy ?? model.guestPolicy).keepOriginals }
  var body: some View {
    Section("Storage") {
      LabeledContent("Unsynced photos", value: size(usage.unsyncedPhotos))
      LabeledContent("Kept originals", value: size(usage.keptOriginals))
      LabeledContent("Offline voyages", value: size(usage.offlineVoyages))
      LabeledContent("Temporary image cache", value: size(usage.temporaryImageCache))
      Toggle(
        "Keep uploaded originals on this device",
        isOn: Binding(
          get: { keepOriginals },
          set: { keep in
            if keep {
              Task { await setKeepOriginals(true) }
            } else {
              changePreference = true
              confirmCleanup = true
            }
          }))
      Text(
        "Pending, failed and uncertain photos are kept. Cleanup requires a ready server photo and a verified local owner copy. The server cannot return your original uploads."
      )
      .font(.footnote)
      if model.workspace != nil {
        NavigationLink("Manage downloads", value: SceneDestination.downloads)
        Button("Remove uploaded originals") {
          changePreference = false
          confirmCleanup = true
        }
        NavigationLink("Export local draft", value: SceneDestination.export)
      }
      Button("Clear temporary cache") {
        Task {
          do {
            try await model.client.imageCache?.clear()
            await refresh()
          } catch { model.error = error.localizedDescription }
        }
      }
      Text(
        "Local drafts are excluded from automatic cloud backup. Deleting the app or losing the device can lose unsynced work."
      )
      .font(.footnote)
    }
    .task { await refresh() }
    .alert("Remove eligible uploaded originals?", isPresented: $confirmCleanup) {
      Button("Remove eligible originals", role: .destructive) {
        Task {
          if changePreference {
            await setKeepOriginals(false)
          } else {
            await model.removeUploadedOriginals()
          }
          await refresh()
        }
      }
      Button("Cancel", role: .cancel) {}
    } message: {
      Text(
        "Export originals you want to keep first. Website variants and offline downloads do not contain the original file or its embedded metadata."
      )
    }
  }
  private func size(_ bytes: Int64) -> String {
    ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
  }
  private func setKeepOriginals(_ keep: Bool) async {
    let account = model.activeAccountId
    await model.setPreference(.keepOriginals, value: keep, accountId: account)
  }
  private func refresh() async {
    do {
      if let account = model.activeAccountId, let storage = model.storage {
        usage = try await storage.breakdown(accountId: account)
      } else {
        usage.temporaryImageCache = try await model.client.imageCache?.byteCount() ?? 0
      }
    } catch { model.error = error.localizedDescription }
  }
}

struct ManageDownloadsView: View {
  @Environment(AppModel.self) private var model
  @State private var removing: LocalVoyage?
  var body: some View {
    List {
      ForEach(model.localVoyages.filter { $0.serverId != nil }) { voyage in
        Section(voyage.fields.title) {
          let photos = model.workspace?.photos(for: voyage.id) ?? []
          let copies = photos.compactMap(\.offlineCopy)
          Text("\(copies.count) of \(photos.count) photos saved for offline use")
          if let date = copies.map(\.savedAt).min() {
            Text("Last saved \(date.formatted())").font(.caption)
          }
          Button(
            model.downloadingVoyages.contains(voyage.id)
              ? "Downloading" : "Download or resume owner copy"
          ) {
            Task { await model.downloadOwnerCopy(voyage.id) }
          }.disabled(
            model.offline || !model.signedIn || model.downloadingVoyages.contains(voyage.id))
          if !copies.isEmpty { Button("Remove download", role: .destructive) { removing = voyage } }
        }
      }
      Text(
        "Downloads include private metadata, thumbnails and display images. Full-size variants download when requested. Basemap tiles may be unavailable offline."
      )
      .font(.footnote)
    }.navigationTitle("Manage downloads")
      .confirmationDialog(
        "Remove this voyage's offline download?",
        isPresented: Binding(
          get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible
      ) {
        Button("Remove download", role: .destructive) {
          if let voyage = removing { Task { await model.removeOwnerCopy(voyage.id) } }
          removing = nil
        }
      } message: {
        Text(
          "Imported originals and website photos are kept. Connect again to download another owner copy."
        )
      }
  }
}

struct ExportVoyagePickerView: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    List {
      ForEach((model.workspace?.voyages ?? []).sorted { $0.fields.title < $1.fields.title }) {
        voyage in
        NavigationLink(voyage.fields.title, value: SceneDestination.exportVoyage(voyage.id))
      }
    }.navigationTitle("Export local draft")
  }
}

struct ExportFilesPicker: UIViewControllerRepresentable {
  let url: URL
  let completed: (URL?) -> Void
  func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
    let picker = UIDocumentPickerViewController(forExporting: [url], asCopy: true)
    picker.delegate = context.coordinator
    return picker
  }
  func updateUIViewController(_ picker: UIDocumentPickerViewController, context: Context) {}
  func makeCoordinator() -> Coordinator { Coordinator(completed: completed) }
  final class Coordinator: NSObject, UIDocumentPickerDelegate {
    let completed: (URL?) -> Void
    init(completed: @escaping (URL?) -> Void) { self.completed = completed }
    func documentPicker(
      _ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]
    ) { completed(urls.first) }
    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { completed(nil) }
  }
}

struct ExportVoyageView: View {
  @Environment(AppModel.self) private var model
  let voyageId: UUID
  @State private var preview: FileVault.ExportPreview?
  @State private var export: URL?
  @State private var exporting = false
  @State private var preparing = false
  var body: some View {
    Form {
      if let preview {
        LabeledContent("Original files included", value: String(preview.includedFiles))
        LabeledContent(
          "Original bytes",
          value: ByteCountFormatter.string(fromByteCount: preview.byteCount, countStyle: .file))
        Text(
          "Voyage metadata, private locations and retained sources for removed photos are included."
        )
        Text("Exact photo locations may be included.").font(.footnote)
        if !preview.missingOriginals.isEmpty {
          Section("Originals unavailable on this device") {
            ForEach(Array(preview.missingOriginals.enumerated()), id: \.offset) { _, name in
              Text(name)
            }
            Text(
              "These originals are listed in the export preview; replacement files are not created."
            ).font(.footnote)
          }
        }
        Button("Prepare export of local work") { Task { await prepare() } }.disabled(preparing)
      }
      if preparing { ProgressView("Preparing export") }
      if let export {
        ShareLink("Export local work", item: export)
        Button("Save export to Files") { exporting = true }
      }
    }.navigationTitle("Export voyage")
      .task {
        guard let workspace = model.workspace else { return }
        do { preview = try await model.vault?.exportPreview(workspace, voyageId: voyageId) } catch {
          model.error = error.localizedDescription
        }
      }
      .sheet(isPresented: $exporting) {
        if let export { ExportFilesPicker(url: export) { _ in exporting = false } }
      }
  }

  private func prepare() async {
    guard let workspace = model.workspace, let vault = model.vault else { return }
    preparing = true
    defer { preparing = false }
    do {
      let url = try await vault.exportArchive(
        workspace, voyageId: voyageId,
        destination: URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
          .appendingPathComponent("PinVoyager-export"))
      export = url
    } catch { model.error = error.localizedDescription }
  }
}
