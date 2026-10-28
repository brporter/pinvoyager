import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct VoyageEditor: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @Environment(\.dismiss) private var dismiss
  @Environment(\.scenePhase) private var scenePhase
  var voyage: LocalVoyage?
  @State private var fields = VoyageFields()
  @State private var session: VoyageEditSession?
  private var publicationArmed: Bool {
    guard let state = model.workspace else { return false }
    return state.publicationIntents.contains {
      $0.voyageId == voyage?.id && state.publicationMayProceed($0)
    }
  }

  @State private var precision = false
  @State private var fieldError: String?
  @State private var saving = false
  @State private var closeConfirmation = false
  @State private var loaded = false
  @State private var persistenceTask: Task<Void, Never>?
  @State private var finished = false

  var body: some View {
    NavigationStack {
      Form {
        if publicationArmed {
          Section {
            Text(
              "The earlier reviewed version may still publish. These edits stay on this iPhone. Cancel scheduled publication in Sync before applying them to the voyage."
            )
          }
        }
        if voyage?.confirmedPublished == true {
          Section { Text("Saved changes will update the public voyage when synchronized.") }
        }
        Section("Voyage details") {
          TextField("Voyage name", text: $fields.title).accessibilityIdentifier("Voyage name")
          if fields.title.unicodeScalars.count > 180 {
            Text("\(fields.title.unicodeScalars.count) / 200 characters").font(.caption)
          }
          TextEditor(
            text: Binding(get: { fields.description ?? "" }, set: { fields.description = $0 })
          )
          .frame(height: 180).accessibilityLabel("Description")
          if (fields.description?.unicodeScalars.count ?? 0) > 7_500 {
            Text("\(fields.description?.unicodeScalars.count ?? 0) / 8,000 characters").font(
              .caption)
          }
        }
        Section {
          Button {
            precision = true
          } label: {
            LabeledContent("Location precision", value: fields.granularity.label)
          }.accessibilityIdentifier("Location precision")
          Text(
            "This controls locations shown publicly. Original photo locations remain available to you. Photos can reveal places through their content too."
          ).font(.footnote)
          if fields.granularity == .exact {
            Label("Exact recorded locations will be public", systemImage: "location.fill")
              .foregroundStyle(.orange)
          }
        }
        if let fieldError { Section { ErrorNotice(message: fieldError) } }
        Section {
          if voyage == nil {
            Button("Add photos") { save(workingCopyOnly: false, addPhotos: true) }
              .disabled(saving).frame(maxWidth: .infinity, minHeight: 44)
              .buttonStyle(.borderedProminent)
          }
          Button(
            voyage == nil
              ? "Save draft and close"
              : publicationArmed ? "Save working copy on this iPhone" : "Save changes"
          ) { save(workingCopyOnly: publicationArmed) }
          .disabled(saving).frame(maxWidth: .infinity, minHeight: 44).buttonStyle(
            .borderedProminent)
          if voyage == nil {
            Text(
              "Your draft is saved on this iPhone before synchronization. Publication requires a separate review."
            ).font(.footnote)
          }
        }
      }
      .navigationTitle(voyage == nil ? "New voyage" : "Edit voyage")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button(voyage == nil ? "Close" : "Cancel") {
            if dirty { closeConfirmation = true } else { dismiss() }
          }
        }
      }
      .confirmationDialog(
        voyage == nil ? "Keep this draft?" : "Discard these edits?",
        isPresented: $closeConfirmation, titleVisibility: .visible
      ) {
        if voyage == nil {
          Button("Keep draft") {
            Task {
              if await persist(fields) {
                finished = true
                persistenceTask?.cancel()
                dismiss()
              }
            }
          }
        }
        Button(voyage == nil ? "Discard draft" : "Discard edits", role: .destructive) {
          persistenceTask?.cancel()
          Task {
            do {
              try await session?.discardWorkingCopy()
              finished = true
              await model.reloadWorkspace()
              dismiss()
            } catch { fieldError = error.localizedDescription }
          }
        }
        Button("Continue editing", role: .cancel) {}
      }
    }
    .sheet(isPresented: $precision) {
      PrecisionPickerView(
        initial: fields.granularity,
        coordinate: voyage.flatMap { saved in
          model.workspace?.photos(for: saved.id).compactMap(\.coordinate).first
        }
      ) {
        fields.granularity = $0
      }
    }
    .interactiveDismissDisabled(dirty)
    .onAppear {
      guard !loaded, let account = model.activeAccountId else { return }
      session = VoyageEditSession(
        accountId: account, voyageId: voyage?.id, repository: model.repository)
      if let voyage {
        fields =
          model.workspace?.workingCopies.first(where: { $0.voyageId == voyage.id })?.fields
          ?? voyage.fields
      } else {
        fields = model.workspace?.scratch ?? VoyageFields()
      }
      loaded = true
      registerNavigationSave()
    }
    .onChange(of: fields) { _, value in
      guard loaded else { return }
      registerNavigationSave()
      persistenceTask?.cancel()
      persistenceTask = Task {
        do {
          try await Task.sleep(for: .milliseconds(300))
          _ = await persist(value)
        } catch is CancellationError {} catch { fieldError = error.localizedDescription }
      }
    }
    .onChange(of: scenePhase) { _, phase in
      if phase != .active {
        persistenceTask?.cancel()
        Task { await persist(fields) }
      }
    }
    .onDisappear {
      router.beforeNavigation = nil
      persistenceTask?.cancel()
      if !finished && dirty { Task { await persist(fields) } }
    }
  }

  private func registerNavigationSave() {
    let value = fields
    let session = session
    router.beforeNavigation = {
      guard let session, model.activeAccountId == session.accountId else {
        throw WorkspaceError.locked
      }
      persistenceTask?.cancel()
      try await session.saveWorkingCopy(value)
      await model.reloadWorkspace()
      dismiss()
    }
  }

  private var dirty: Bool {
    voyage.map { $0.fields != fields }
      ?? (!fields.title.isEmpty || !(fields.description ?? "").isEmpty)
  }
  @discardableResult private func persist(_ value: VoyageFields) async -> Bool {
    guard !finished, !Task.isCancelled, let session, model.activeAccountId == session.accountId
    else { return false }
    do {
      try await session.saveWorkingCopy(value)
      await model.reloadWorkspace()
      return true
    } catch {
      fieldError = error.localizedDescription
      return false
    }
  }
  private func save(workingCopyOnly: Bool, addPhotos: Bool = false) {
    guard let session, model.activeAccountId == session.accountId else { return }
    saving = true
    persistenceTask?.cancel()
    Task {
      defer { saving = false }
      do {
        let savedId: UUID?
        if workingCopyOnly {
          try await session.saveWorkingCopy(fields)
          savedId = voyage?.id
        } else {
          savedId = try await session.commit(fields)
        }
        await model.reloadWorkspace()
        guard model.activeAccountId == session.accountId else { return }
        if voyage == nil, let savedId {
          router.creationCompleted(voyageId: savedId, addPhotos: addPhotos)
        }
        finished = true
        dismiss()
        await model.sync()
      } catch { fieldError = error.localizedDescription }
    }
  }
}
