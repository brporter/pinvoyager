import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct ConflictResolutionView: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let operation: OutboxOperation
  private enum PhotoReview: String, CaseIterable {
    case append = "Use website order and append my new photos"
    case arrange = "Arrange combined photos"
    case website = "Keep website version"
  }
  @State private var review: ConflictReview?
  @State private var finished = false
  @State private var saving = false
  @State private var savedLocally = false
  @State private var photoReview: PhotoReview?
  @State private var combined: [PhotoConflictItem] = []
  @State private var readded: Set<UUID> = []
  @State private var publicConsent = false
  private var requiresPhotoReview: Bool {
    switch operation.kind {
    case .order, .removePhoto: true
    default: false
    }
  }
  @State private var makePrivate: Bool?
  @State private var persistenceTask: Task<Void, Never>?
  private var requiresVisibilityReview: Bool {
    if case .unpublish = operation.kind { true } else { false }
  }
  @State private var choices: [String: FieldResolution] = [:]
  @State private var discardConfirmation = false
  @State private var unavailable = false
  @State private var error: String?
  private var voyage: LocalVoyage? {
    model.workspace?.voyages.first { $0.id == operation.voyageId }
  }

  var body: some View {
    Form {
      if voyage != nil, let review {
        Section {
          Text(
            "This voyage changed on the website. Choose which values to keep. Your saved photo files are preserved."
          )
        }
        if requiresVisibilityReview {
          Section("Website visibility") {
            VisibilityLabel(published: review.published)
            Picker("Visibility resolution", selection: $makePrivate) {
              Text("Choose a version").tag(Bool?.none)
              Text("Keep website visibility").tag(Optional(false))
              Text("Make voyage private").tag(Optional(true))
            }
            Text(
              "Making this voyage private requires a new revision-checked request. It remains public until the server confirms that request."
            )
            .font(.footnote)
          }
        } else {
          field(
            "title", label: "Voyage name", mine: review.mine.title, theirs: review.website.title)
          field(
            "description", label: "Description", mine: review.mine.description ?? "",
            theirs: review.website.description ?? "")
          field(
            "granularity", label: "Public location precision", mine: review.mine.granularity.label,
            theirs: review.website.granularity.label)
        }
        if !requiresVisibilityReview {
          Section("Public location preview") {
            PrecisionPreview(
              granularity: workingReview?.resolvedFields.granularity ?? review.website.granularity,
              coordinate: model.workspace?.photos(for: review.voyageId).compactMap(\.coordinate)
                .first)
          }
        }
        if requiresPhotoReview { photoComparison(review) }
        Button(model.offline ? "Save resolution on this iPhone" : "Save resolution") { save() }
          .disabled(
            saving
              || (!model.offline
                && ((requiresVisibilityReview
                  ? makePrivate == nil : workingReview?.fieldsChosen != true)
                  || (requiresPhotoReview && photoReview == nil)))
          )
        if savedLocally { Label("Resolution saved on this iPhone", systemImage: "iphone") }
        if model.offline {
          Text(
            "Your choices can be saved locally. Connect to submit a revision-checked resolution.")
        }
        if let error { ErrorNotice(message: error) }
      } else if unavailable {
        Section {
          Text(
            "The destination is unavailable. A missing voyage may have been deleted or its access may have changed. Your saved work remains on this device."
          )
          Button("Keep as a new private voyage") {
            if let account = model.activeAccountId, let id = voyage?.id, let vault = model.vault {
              Task {
                do {
                  _ = try await DraftRecoveryService(store: model.store, vault: vault).recover(
                    accountId: account, voyageId: id)
                  await model.reloadWorkspace()
                  model.scheduleSync()
                  dismiss()
                } catch { self.error = error.localizedDescription }
              }
            }
          }
          Text(
            "Recovery includes verified available originals or processed owner downloads. Processed copies cannot restore the original file or its embedded capture metadata."
          ).font(.footnote)
          NavigationLink("Export local work", value: SceneDestination.export)
          Button("Discard local work", role: .destructive) { discardConfirmation = true }
          if let error { ErrorNotice(message: error) }
        }
      } else if let error {
        ErrorNotice(message: error) { Task { await load() } }
      } else {
        ProgressView("Loading website version")
      }
    }.navigationTitle("Resolve changes").task { await load() }
      .confirmationDialog(
        "Discard this voyage's local files and edits?", isPresented: $discardConfirmation,
        titleVisibility: .visible
      ) {
        Button("Discard files and edits", role: .destructive) {
          guard let account = model.activeAccountId, let id = voyage?.id,
            let discard = model.localDiscard
          else { return }
          Task {
            do {
              try await discard.discardUnavailable(accountId: account, voyageId: id)
              await model.reloadWorkspace()
              dismiss()
            } catch { self.error = error.localizedDescription }
          }
        }
      } message: {
        Text(
          "Export anything you want to keep first. This removes local sources and working copies and stops their pending work. It does not delete another website voyage."
        )
      }
      .toolbar { if photoReview == .arrange { EditButton() } }
      .onChange(of: workingReview) { _, value in
        guard !finished, !saving, let value, let account = model.activeAccountId else { return }
        let previous = persistenceTask
        previous?.cancel()
        persistenceTask = Task {
          await previous?.value
          guard !Task.isCancelled else { return }
          do { try await service.save(value, accountId: account, operationId: operation.id) } catch
          { self.error = error.localizedDescription }
        }
      }
  }

  @ViewBuilder private func field(_ key: String, label: String, mine: String, theirs: String)
    -> some View
  {
    Section(label) {
      LabeledContent("Your saved version", value: mine)
      LabeledContent("Website version", value: theirs)
      Picker(
        label, selection: Binding(get: { choices[key] ?? .needsChoice }, set: { choices[key] = $0 })
      ) {
        Text("Choose a version").tag(FieldResolution.needsChoice)
        Text("Use mine").tag(FieldResolution.local)
        Text("Use website").tag(FieldResolution.remote)
      }
    }
  }
  @ViewBuilder private func photoComparison(_ review: ConflictReview) -> some View {
    let photos = model.workspace?.photos(for: voyage?.id ?? operation.id) ?? []
    let websiteIDs = Set(review.websitePhotos)
    Section("Your saved photo order") {
      ForEach(photos) { photo in
        VStack(alignment: .leading) {
          Text(photo.filename)
          if let id = photo.serverId, !websiteIDs.contains(id) {
            Text("Deleted on the website").font(.caption)
          } else if photo.serverId == nil {
            Text("Added on this iPhone").font(.caption)
          }
        }
      }
    }
    Section("Website photo order") {
      ForEach(review.websitePhotos, id: \.self) { id in
        Text(photoName(.website(id)))
      }
    }
    Section("Resolve photo membership and order") {
      Picker("Photo resolution", selection: $photoReview) {
        Text("Choose a photo resolution").tag(PhotoReview?.none)
        ForEach(PhotoReview.allCases, id: \.self) { Text($0.rawValue).tag(Optional($0)) }
      }
      Text("Website deletions stay deleted. Local original files remain available in your export.")
    }
    if photoReview == .arrange {
      Section("Combined photo order") {
        ForEach(combined, id: \.self) { Text(photoName($0)) }
          .onMove { combined.move(fromOffsets: $0, toOffset: $1) }
      }
    }
    let removed = photos.filter {
      $0.serverId.map { !websiteIDs.contains($0) } == true && $0.source != nil
    }
    if !removed.isEmpty {
      Section("Retained sources for website deletions") {
        if review.published {
          Toggle("Added photos will become public as they upload", isOn: $publicConsent)
        }
        ForEach(removed) { photo in
          VStack(alignment: .leading) {
            Text(photo.filename)
            Button(
              readded.contains(photo.id)
                ? "Added with a new upload identity" : "Add again as a new photo"
            ) {
              guard let account = model.activeAccountId else { return }
              Task {
                do {
                  let id = try await model.repository.addAgain(
                    accountId: account, photoId: photo.id, publicConsent: publicConsent)
                  combined.append(.local(id))
                  readded.insert(photo.id)
                  await model.reloadWorkspace()
                } catch { self.error = error.localizedDescription }
              }
            }.disabled(readded.contains(photo.id) || (review.published && !publicConsent))
          }
        }
      }
    }
  }
  private func photoName(_ item: PhotoConflictItem) -> String {
    let photo = model.workspace?.photos.first { photo in
      switch item {
      case .website(let id): photo.serverId == id && photo.voyageId == operation.voyageId
      case .local(let id): photo.id == id
      }
    }
    if let photo { return photo.filename }
    if case .website(let id) = item,
      let position = review?.websitePhotos.firstIndex(of: id)
    {
      return "Website photo \(position + 1)"
    }
    return "Saved photograph"
  }
  private var service: ConflictReviewService {
    ConflictReviewService(store: model.store, remote: model.client)
  }
  private var workingReview: ConflictReview? {
    guard var review else { return nil }
    review.choices = choices
    review.makePrivate = makePrivate
    review.readded = readded
    switch photoReview {
    case .append: review.photoChoice = .websiteAppendingLocal
    case .website: review.photoChoice = .website
    case .arrange: review.photoChoice = .arranged(combined)
    case nil: review.photoChoice = nil
    }
    return review
  }
  private func display(_ value: ConflictReview) {
    review = value
    makePrivate = value.makePrivate
    choices = value.choices
    readded = value.readded
    switch value.photoChoice {
    case .website: photoReview = .website
    case .websiteAppendingLocal: photoReview = .append
    case .arranged: photoReview = .arrange
    case nil: photoReview = nil
    }
    if case .arranged(let items) = value.photoChoice {
      combined = items
    } else {
      combined =
        value.websitePhotos.map(PhotoConflictItem.website)
        + (model.workspace?.photos(for: value.voyageId).filter { $0.serverId == nil } ?? []).map {
          .local($0.id)
        }
    }
  }
  private func load() async {
    guard voyage?.serverId != nil, let account = model.activeAccountId else {
      error = "No remote voyage is mapped. Export local work or keep it as a new private voyage."
      return
    }
    let access = model.publicAccess
    do {
      let value = try await service.load(
        accountId: account, operationId: operation.id, online: !model.offline)
      guard access == model.publicAccess, !Task.isCancelled else { return }
      display(value)
    } catch {
      guard access == model.publicAccess, !Task.isCancelled else { return }
      if let error = error as? APIError, error.status == 404 {
        unavailable = true
      } else {
        self.error = error.localizedDescription
      }
    }
  }
  private func save() {
    guard let value = workingReview, let account = model.activeAccountId else { return }
    saving = true
    error = nil
    Task {
      defer { saving = false }
      do {
        await persistenceTask?.value
        switch try await service.apply(
          value, accountId: account, operationId: operation.id, online: !model.offline)
        {
        case .savedLocally:
          savedLocally = true
          await model.reloadWorkspace()
        case .needsReview(let latest):
          display(latest)
          error =
            "The website changed again. Your choices were kept. Review the updated version before saving."
        case .resolved:
          finished = true
          await model.reloadWorkspace()
          dismiss()
          await model.sync()
        }
      } catch { self.error = error.localizedDescription }
    }
  }
}
