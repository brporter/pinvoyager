import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct LocalVoyageView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @Environment(\.scenePhase) private var phase
  let id: UUID
  @State private var managing = false
  @State private var editing = false
  @State private var importing = false
  @State private var publication = false
  @State private var privateLocations = false
  @State private var unpublishConfirmation = false
  @State private var galleryPhoto: LocalPhoto?
  @State private var galleryMembers: Set<UUID>?
  @State private var galleryAnchor: UUID?
  @State private var selectedMapPhoto: UUID?
  @AccessibilityFocusState private var focusedPhoto: UUID?
  @State private var recovering: LocalPhoto?
  @State private var removing: LocalPhoto?
  @State private var removeDownload = false
  @State private var lastRemovedPhoto: UUID?
  private var publicationArmed: Bool {
    guard let state = model.workspace else { return false }
    return state.publicationIntents.contains {
      $0.voyageId == id && state.publicationMayProceed($0)
    }
  }

  var body: some View {
    if let voyage = model.workspace?.voyages.first(where: { $0.id == id && !$0.deleted }) {
      List {
        Section {
          VoyageHeading(title: voyage.fields.title, description: voyage.fields.description)
          VisibilityLabel(published: voyage.confirmedPublished)
          Label("Saved on this iPhone", systemImage: "iphone")
          Text(voyage.fields.granularity.label).font(.subheadline)
          if publicationArmed {
            Text(
              model.workspace?.publicationIntents.contains { $0.voyageId == id && $0.isArmed }
                == true
                ? "Publication scheduled. The reviewed version may publish while this device is offline."
                : "Checking whether publication was scheduled. The reviewed version may publish while this device is offline."
            )
          }
        }
        Section {
          Toggle("Private locations (only you)", isOn: $privateLocations)
          VoyageMapView(
            pins: photos.compactMap { photo in
              photo.coordinate.map {
                MapPhotoPin(
                  id: photo.id,
                  coordinate: privateLocations ? $0 : voyage.fields.granularity.apply($0))
              }
            }, selectedPhotoId: selectedMapPhoto
          ) { members in
            galleryMembers = Set(members)
            galleryPhoto = photos.first { members.contains($0.id) }
            galleryAnchor = galleryPhoto?.id
          }
          Text(
            privateLocations
              ? "Exact recorded locations — private metadata" : "Public location preview"
          ).font(.footnote)
        }
        Section(
          "\(photos.count) photos · \(photos.filter { $0.coordinate == nil }.count) without location"
        ) {
          ForEach(photos) { photo in
            Button {
              galleryMembers = nil
              galleryAnchor = photo.id
              galleryPhoto = photo
            } label: {
              HStack {
                if let remote = photo.serverId, photo.source == nil, photo.offlineCopy == nil {
                  RemoteImage(path: "/api/photos/\(remote)/thumb", accountId: model.activeAccountId)
                    .frame(width: 72, height: 72)
                } else {
                  LocalPhotoImage(photo: photo).frame(width: 72, height: 72)
                }
                VStack(alignment: .leading) {
                  Text(photo.filename).foregroundStyle(.primary)
                  Text(
                    photo.variant == .failed
                      ? "Could not process"
                      : photo.serverId != nil && photo.variant == .pending
                        ? "Processing photo" : photo.transfer.label
                  ).font(
                    .caption
                  ).foregroundStyle(.secondary)
                  if photo.coordinate == nil {
                    Text("No recorded location").font(.caption).foregroundStyle(.secondary)
                  }
                }
              }
            }.swipeActions {
              Button("Replace source") { recovering = photo }.disabled(publicationArmed)
              Button("Remove", role: .destructive) { removing = photo }.disabled(publicationArmed)
            }.contextMenu {
              Button("Recover or replace source") { recovering = photo }.disabled(publicationArmed)
            }.accessibilityFocused($focusedPhoto, equals: photo.id)
          }
          Button("Manage photos") { managing = true }.disabled(publicationArmed)
          Button("Add photos", systemImage: "plus") { importing = true }.disabled(publicationArmed)
        }
        Section {
          if !voyage.confirmedPublished {
            Button("Review publication") { publication = true }.disabled(publicationArmed)
          } else {
            Button("Make voyage private") { unpublishConfirmation = true }
          }
          if let remote = voyage.serverId {
            if voyage.confirmedPublished {
              ShareLink(
                "Share voyage", item: model.webOrigin.url(path: ["voyages", remote.uuidString]))
            }
            NavigationLink("View website version", value: SceneDestination.voyage(remote))
          }
          if voyage.serverId != nil {
            if photos.contains(where: { $0.offlineCopy != nil }) {
              Label("Website copy available offline", systemImage: "arrow.down.circle.fill")
              Button("Remove downloaded copy", role: .destructive) { removeDownload = true }
            }
            Button(
              model.downloadingVoyages.contains(id)
                ? "Downloading website copy…" : "Download website copy"
            ) {
              Task { await model.downloadOwnerCopy(id) }
            }.disabled(
              model.offline || !model.signedIn || model.downloadingVoyages.contains(id)
                || model.capabilities?.ownerOfflineDownloads != true)
            Text(
              "Downloads contain processed photos, never originals. Offline maps still need a connection."
            ).font(.footnote)
          }
          NavigationLink("Sync details", value: SceneDestination.voyageSync(id))
        }
      }
      .navigationTitle(voyage.fields.title).navigationBarTitleDisplayMode(.inline)
      .safeAreaInset(edge: .bottom) {
        if let photoId = lastRemovedPhoto,
          model.workspace?.canUndoPhotoRemoval([photoId]) == true
        {
          HStack {
            Text("Photo removed")
            Spacer()
            Button("Undo removal") {
              guard let accountId = model.activeAccountId else { return }
              Task {
                await model.perform {
                  try await model.repository.undoPhotoRemoval(
                    accountId: accountId, photoId: photoId)
                }
                lastRemovedPhoto = nil
              }
            }
          }.padding().background(.regularMaterial)
        }
      }
      .toolbar {
        Button("Edit details") { editing = true }.disabled(
          voyage.serverId != nil && voyage.revision == nil)
      }
      .task(
        id: LocalRefreshRequest(voyageId: id, access: model.publicAccess, active: phase == .active)
      ) {
        guard !AppBootstrap.isUITesting, phase == .active else { return }
        try? await ForegroundPolling.observe { () -> [String]? in
          guard !model.offline, model.signedIn, phase == .active else { return nil }
          await model.sync()
          await model.refreshVoyage(id)
          let intents = model.workspace?.publicationIntents.filter { $0.voyageId == id } ?? []
          guard
            photos.contains(where: { $0.serverId != nil && $0.variant == .pending })
              || intents.contains(where: \.isArmed)
          else { return nil }
          return photos.map { "\($0.id):\($0.variant.rawValue)" }
            + intents.map { "\($0.id):\($0.status.rawValue)" }
        }
      }
      .onChange(of: router.ownerAction, initial: true) { _, action in
        switch action {
        case .edit(let target) where target == id:
          editing = true
          router.ownerAction = nil
        case .addPhotos(let target) where target == id:
          importing = true
          router.ownerAction = nil
        default: break
        }
      }
      .sheet(isPresented: $managing) { ManagePhotosView(voyage: voyage) }
      .sheet(isPresented: $editing) { VoyageEditor(voyage: voyage) }
      .sheet(item: $recovering) { PhotoRecoveryView(photo: $0) }
      .sheet(isPresented: $importing) { PhotoImportView(voyage: voyage) }
      .sheet(isPresented: $publication) { PublicationReview(voyageId: id) }
      .fullScreenCover(item: $galleryPhoto, onDismiss: { focusedPhoto = galleryAnchor }) { photo in
        OwnerGallery(voyageId: id, selectedId: photo.id, members: galleryMembers) {
          selectedMapPhoto = $0
        }
      }
      .confirmationDialog(
        "Remove downloaded copy?", isPresented: $removeDownload, titleVisibility: .visible
      ) {
        Button("Remove download", role: .destructive) { Task { await model.removeOwnerCopy(id) } }
      } message: {
        Text("Your website photos and locally imported originals remain saved.")
      }
      .confirmationDialog(
        "Remove this photograph from the voyage?",
        isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
        titleVisibility: .visible
      ) {
        Button("Remove from voyage", role: .destructive) {
          if let photoId = removing?.id, let accountId = model.activeAccountId {
            Task {
              await model.perform {
                try await model.repository.removePhoto(accountId: accountId, photoId: photoId)
                lastRemovedPhoto = photoId
              }
            }
          }
          removing = nil
        }
      } message: {
        Text(
          "If uploaded, the photograph will be deleted from the website when synchronized. Its source is retained until the outcome is known."
        )
      }
      .confirmationDialog(
        "Make voyage private?", isPresented: $unpublishConfirmation, titleVisibility: .visible
      ) {
        Button("Make voyage private") {
          if let accountId = model.activeAccountId {
            Task {
              await model.perform {
                try await model.repository.unpublish(accountId: accountId, voyageId: id)
              }
              await model.sync()
            }
          }
        }
      } message: {
        Text("The voyage remains public until the server confirms unpublication.")
      }
    } else {
      ContentUnavailableView("This voyage is unavailable", systemImage: "suitcase")
    }
  }
  private var photos: [LocalPhoto] { model.workspace?.photos(for: id) ?? [] }

}

struct PublicationReview: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let voyageId: UUID
  @State private var accepted = false
  @State private var queued = false
  var body: some View {
    NavigationStack {
      Form {
        if let voyage = model.workspace?.voyages.first(where: { $0.id == voyageId }) {
          Section {
            VoyageHeading(title: voyage.fields.title, description: voyage.fields.description)
            Text(voyage.fields.granularity.label)
            if voyage.fields.granularity == .exact {
              Label("Exact recorded locations will be public", systemImage: "location.fill")
            }
            Text("\(model.workspace?.photos(for: voyageId).count ?? 0) photos")
            Text(
              "Anyone with the link can view this voyage. It can also appear in Discover and your public profile."
            )
          }
          Section {
            if queued {
              Label("Publication queued on this iPhone", systemImage: "clock")
              Text("This voyage has not been published. Consent expires after seven days.")
            } else {
              Toggle("I approve this exact photo list and location precision", isOn: $accepted)
              Text(
                "The server publishes only after every reviewed photograph is ready. Changes require a fresh review. Consent expires in seven days."
              ).font(.footnote)
              if !model.nativeSupported {
                Text(
                  "This server does not yet support scheduled publication. You can save your review locally; it will remain unpublished."
                ).font(.footnote)
              }
              Button(
                model.offline || !model.nativeSupported ? "Queue publication" : "Publish when ready"
              ) { queue() }.disabled(!accepted || validationMessage != nil)
              if let validationMessage { Text(validationMessage).foregroundStyle(.secondary) }
            }
          }
        }
      }.navigationTitle("Publication review").toolbar { Button("Close") { dismiss() } }
    }
  }
  private var validationMessage: String? {
    guard let state = model.workspace,
      let voyage = state.voyages.first(where: { $0.id == voyageId })
    else { return "Open a saved voyage to review publication." }
    do {
      _ = try PublicationIntent.review(
        voyage: voyage, photos: state.photos(for: voyageId), at: Date())
      return nil
    } catch { return error.localizedDescription }
  }
  private func queue() {
    guard let accountId = model.activeAccountId else { return }
    Task {
      do {
        _ = try await model.repository.queuePublication(accountId: accountId, voyageId: voyageId)
        queued = true
        await model.reloadWorkspace()
        await model.sync()
      } catch { model.error = error.localizedDescription }
    }
  }
}

private struct LocalRefreshRequest: Hashable {
  let voyageId: UUID
  let access: PublicAccess
  let active: Bool
}
