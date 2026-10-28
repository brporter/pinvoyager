import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct ManagePhotosView: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let voyage: LocalVoyage
  @State private var arrangement = PhotoArrangement(ids: [])
  @State private var selecting = false
  @State private var reordering = false
  @State private var importing = false
  @State private var removalConfirmation = false
  @State private var removed: Set<UUID> = []
  @State private var saving = false
  @State private var error: String?
  private var photos: [LocalPhoto] { model.workspace?.photos(for: voyage.id) ?? [] }
  private var ids: [UUID] { photos.map(\.id) }
  private var armed: Bool {
    guard let state = model.workspace else { return false }
    return state.publicationIntents.contains {
      $0.voyageId == voyage.id && state.publicationMayProceed($0)
    }
  }

  var body: some View {
    NavigationStack {
      List {
        Section {
          if voyage.confirmedPublished {
            Text("Saved order and removals will change the public voyage when synchronized.")
          }
          if reordering {
            Text(
              "Move photos, then save the complete order. New arrivals append to your arrangement.")
          } else if selecting {
            Text("\(arrangement.selected.count) selected")
          }
          if let error { ErrorNotice(message: error) }
        }
        ForEach(arrangement.ids, id: \.self) { id in
          if let photo = photos.first(where: { $0.id == id }) {
            HStack {
              if selecting {
                Button {
                  arrangement.toggleSelection(id)
                } label: {
                  Image(
                    systemName: arrangement.selected.contains(id)
                      ? "checkmark.circle.fill" : "circle"
                  )
                  .frame(width: 44, height: 44).contentShape(Rectangle())
                }.accessibilityLabel("Select \(photo.filename)")
                  .accessibilityValue(
                    arrangement.selected.contains(id) ? "Selected" : "Not selected")
              }
              LocalPhotoImage(photo: photo).frame(width: 52, height: 52)
              Text(photo.filename)
              if reordering {
                Spacer()
                Button {
                  arrangement.move(id, by: -1)
                } label: {
                  Image(systemName: "arrow.up").frame(width: 44, height: 44).contentShape(
                    Rectangle())
                }
                .accessibilityLabel("Move \(photo.filename) earlier")
                .disabled(arrangement.ids.first == id)
                Button {
                  arrangement.move(id, by: 1)
                } label: {
                  Image(systemName: "arrow.down").frame(width: 44, height: 44).contentShape(
                    Rectangle())
                }
                .accessibilityLabel("Move \(photo.filename) later")
                .disabled(arrangement.ids.last == id)
              }
            }.buttonStyle(.borderless)
          }
        }.onMove { indices, destination in
          var order = arrangement.ids
          order.move(fromOffsets: indices, toOffset: destination)
          arrangement.setOrder(order)
        }
        Section {
          if reordering {
            Button("Save order") {
              let order = arrangement.ids
              commit {
                try await model.repository.reorder(
                  accountId: voyage.ownerId, voyageId: voyage.id, photoIds: order)
                reordering = false
              }
            }.disabled(saving || arrangement.ids == ids || armed)
            Button("Cancel order changes", role: .cancel) {
              arrangement.reset(ids)
              reordering = false
            }
          } else {
            Button(selecting ? "Done selecting" : "Select") {
              selecting.toggle()
              arrangement.reset(ids)
            }.disabled(armed)
            if selecting {
              Button("Remove selected", role: .destructive) { removalConfirmation = true }
                .disabled(arrangement.selected.isEmpty || saving || armed)
            } else {
              Button("Reorder") { reordering = true }.disabled(photos.count < 2 || armed)
              Button("Add photos", systemImage: "plus") { importing = true }.disabled(armed)
            }
          }
        }
      }
      .environment(\.editMode, .constant(reordering ? .active : .inactive))
      .navigationTitle("Manage photos").navigationBarTitleDisplayMode(.inline)
      .toolbar { Button("Done") { dismiss() }.disabled(reordering || saving) }
      .onAppear { arrangement.reset(ids) }
      .onChange(of: ids) { _, value in
        arrangement.reconcile(value)
        if !reordering { arrangement.setOrder(value) }
      }
      .safeAreaInset(edge: .bottom) {
        if model.workspace?.canUndoPhotoRemoval(removed) == true {
          HStack {
            Text("Photos removed")
            Spacer()
            Button("Undo removals") {
              let ids = removed
              commit {
                try await model.repository.undoPhotoRemovals(
                  accountId: voyage.ownerId, voyageId: voyage.id, photoIds: ids)
                removed = []
              }
            }.disabled(saving)
          }.padding().background(.regularMaterial)
        }
      }
      .interactiveDismissDisabled(reordering || saving)
      .sheet(isPresented: $importing) { PhotoImportView(voyage: voyage) }
      .confirmationDialog(
        "Remove \(arrangement.selected.count) photographs?",
        isPresented: $removalConfirmation, titleVisibility: .visible
      ) {
        Button("Remove \(arrangement.selected.count) photographs", role: .destructive) {
          let selected = arrangement.selected
          commit {
            try await model.repository.removePhotos(
              accountId: voyage.ownerId, voyageId: voyage.id, photoIds: selected)
            removed = selected
          }
        }
      } message: {
        Text(
          "Uploaded photos will be deleted from this voyage and the website when synchronized. Sources stay saved until uncertain uploads and removals have been checked."
        )
      }
    }
  }

  private func commit(_ command: @escaping @MainActor () async throws -> Void) {
    guard model.activeAccountId == voyage.ownerId else { return }
    saving = true
    error = nil
    Task {
      defer { saving = false }
      do {
        try await command()
        await model.cancelIneligibleTransfers?(voyage.ownerId)
        await model.reloadWorkspace()
        await model.sync()
      } catch { self.error = error.localizedDescription }
    }
  }
}
