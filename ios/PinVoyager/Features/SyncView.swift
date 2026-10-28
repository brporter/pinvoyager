import PinVoyagerDomain
import SwiftUI

struct SyncView: View {
  @Environment(AppModel.self) private var model
  var voyageId: UUID?
  var body: some View {
    List {
      if let workspace = model.workspace {
        if !model.nativeSupported {
          Section {
            Text(
              "Native synchronization is not supported by this server yet. Your saved work remains on this device."
            )
          }
        }
        Section {
          Button(model.syncing ? "Syncing" : "Sync now", systemImage: "arrow.triangle.2.circlepath")
          { Task { await model.sync() } }.disabled(
            model.syncing || model.offline || !model.nativeSupported)
          if let voyageId, let voyage = workspace.voyages.first(where: { $0.id == voyageId }) {
            NavigationLink("Open voyage", value: SceneDestination.localVoyage(voyageId))
            Button(
              voyage.uploadsPaused == true
                ? "Resume uploads for voyage" : "Pause uploads for voyage"
            ) {
              Task {
                await model.perform {
                  try await model.repository.setUploadsPaused(
                    accountId: workspace.account.id, voyageId: voyageId,
                    paused: voyage.uploadsPaused != true)
                }
              }
            }
            if voyage.uploadsPaused == true {
              Text(
                "Uploads paused for this voyage. Receipt checks and other changes can still synchronize."
              )
            }
          }
          if voyageId == nil {
            Button(workspace.policy.areUploadsPaused ? "Resume uploads" : "Pause uploads") {
              Task {
                await model.setPreference(
                  .uploadsPaused, value: !workspace.policy.areUploadsPaused,
                  accountId: workspace.account.id)
              }
            }
            if workspace.policy.areUploadsPaused {
              Text(
                "Photo uploads are paused. Receipt checks, metadata, and privacy changes can still synchronize."
              )
            }
          }
          NavigationLink("Sync and storage settings", value: SceneDestination.preferences)
        }
        if let voyageId, let voyage = workspace.voyages.first(where: { $0.id == voyageId }) {
          Section("Voyage progress") {
            Text(voyage.fields.title).font(.headline)
            VisibilityLabel(published: voyage.confirmedPublished)
            let progress = VoyageSyncProgress(voyage: voyage, workspace: workspace)
            ForEach(progress.milestones) { milestone in
              Label(
                milestone.title,
                systemImage: milestone.complete ? "checkmark.circle.fill" : "circle"
              )
              .accessibilityValue(milestone.complete ? "Confirmed" : "Waiting")
            }
            Text(
              "\(progress.acceptedCount) of \(progress.photoCount) photos accepted · \(progress.processedCount) processed"
            )
          }
        }
        ForEach(workspace.publicationIntents.filter { voyageId == nil || $0.voyageId == voyageId })
        { intent in
          Section("Publication") {
            Text(intent.fields.title).font(.headline)
            Text(
              intent.status == .queued
                ? "Queued on this iPhone — not published"
                : intent.status.rawValue.replacingOccurrences(of: "_", with: " ").capitalized)
            Text("Consent expires \(intent.expiresAt.formatted())").font(.caption)
            if intent.cancellationPending {
              Text("Cancellation is waiting for a connection. This voyage may still publish.")
            }
            if intent.status == .queued || intent.isArmed {
              Button("Cancel publication", role: .destructive) {
                Task {
                  await model.perform {
                    try await model.repository.cancelPublication(
                      accountId: workspace.account.id, intentId: intent.id)
                  }
                  await model.sync()
                }
              }
            }
          }
        }
        ForEach(SyncActivity.Section.allCases) { category in
          let rows = activity.rows(for: category)
          if !rows.isEmpty {
            Section(category.rawValue) {
              ForEach(rows) { row in
                if voyageId == nil, let id = row.voyageId,
                  let voyage = workspace.voyages.first(where: { $0.id == id })
                {
                  NavigationLink(voyage.fields.title, value: SceneDestination.voyageSync(id))
                    .accessibilityIdentifier("sync.voyage.\(id)")
                }
                ForEach(row.operations) { operationRow($0, accountId: workspace.account.id) }
              }
            }
          }
        }
      }
      if operations.allSatisfy({ [.applied, .canceled].contains($0.state) }) {
        ContentUnavailableView("No pending work", systemImage: "checkmark.circle")
      }
      Section("Background synchronization") {
        Text(
          "iOS chooses when background transfers run. If you force-quit PinVoyager, reopen it to resume. No background completion deadline is promised."
        )
      }
    }.navigationTitle(voyageId == nil ? "Sync" : "Voyage sync details")
  }
  private var operations: [OutboxOperation] {
    (model.workspace?.operations ?? []).filter { voyageId == nil || $0.voyageId == voyageId }
  }
  private var activity: SyncActivity { SyncActivity(operations: operations) }
  @ViewBuilder private func operationRow(_ operation: OutboxOperation, accountId: UUID) -> some View
  {
    VStack(alignment: .leading, spacing: 8) {
      Text(operationTitle(operation.kind)).font(.headline)
      Text(operation.state.rawValue.capitalized)
      if let error = operation.lastError {
        Text(error).font(.footnote).foregroundStyle(.secondary)
      }
      if let requestId = operation.requestId {
        Text("Reference: \(requestId)").font(.caption).textSelection(.enabled)
      }
      if operation.state == .attention {
        Button("Retry") {
          Task {
            await model.perform {
              try await model.repository.retry(
                accountId: accountId, operationId: operation.id)
            }
            await model.sync()
          }
        }.accessibilityIdentifier("sync.retry.\(operation.id)")
      }
      if operation.state == .conflict {
        switch operation.kind {
        case .edit, .order, .removePhoto, .unpublish:
          NavigationLink("Resolve changes", value: SceneDestination.conflict(operation.id))
        default:
          Text(
            "This voyage changed on the website. Your saved version has been preserved. Export your work before resolving photo membership or visibility changes."
          ).font(.footnote)
        }
      }
    }.padding(.vertical, 8)
  }
  private func operationTitle(_ kind: OperationKind) -> String {
    switch kind {
    case .create: "Create draft"
    case .edit: "Save voyage details"
    case .upload: "Upload photograph"
    case .order: "Save photo order"
    case .removePhoto: "Remove photograph"
    case .deleteVoyage: "Delete voyage"
    case .unpublish: "Make voyage private"
    case .publish: "Request publication"
    case .cancelPublication: "Cancel publication"
    case .block: "Update blocked traveler"
    case .report: "Send report"
    }
  }
}

struct SyncSettingsView: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    Form {
      Section("Network") {
        Toggle("Upload photos on Wi-Fi only", isOn: preference(.wifiOnly))
        Toggle("Use Low Data Mode connections", isOn: preference(.allowsConstrained))
        Toggle("Allow scheduled background preparation", isOn: preference(.backgroundPreparation))
        Toggle("Pause photo uploads", isOn: preference(.uploadsPaused))
        Toggle("Pause synchronization", isOn: preference(.paused))
        Text("iOS controls background availability and timing.").font(.footnote)
      }
      if let workspace = model.workspace {
        let batches = Dictionary(
          grouping: workspace.photos.filter {
            !$0.deleted && $0.serverId == nil && $0.importBatchId != nil
          }, by: { $0.importBatchId! }
        ).sorted { $0.key.uuidString < $1.key.uuidString }
        if !batches.isEmpty {
          Section("Pending imports") {
            ForEach(batches, id: \.key) { batch, photos in
              Toggle(
                isOn: Binding(
                  get: { photos.allSatisfy { $0.cellularException == true } },
                  set: { allowed in
                    Task { await model.setCellularException(batchId: batch, allowed: allowed) }
                  }
                )
              ) {
                Text("Use cellular for this batch")
                Text(
                  "\(photos.count) photos · \(ByteCountFormatter.string(fromByteCount: photos.compactMap(\.source?.byteCount).reduce(0, +), countStyle: .file)) saved bytes waiting"
                )
                .font(.caption)
              }
            }
            Text(
              "Permission ends when these uploads are accepted or removed. Low Data Mode and pause settings still apply."
            )
            .font(.footnote)
          }
        }
      }
      Section("Notifications") {
        Toggle(
          "Notify when publication completes",
          isOn: Binding(
            get: { model.workspace?.policy.notifyOnPublication == true },
            set: { enabled in
              Task {
                await model.setPublicationNotifications(enabled)
              }
            }
          )
        ).disabled(model.workspace == nil)
        Text(
          "A local notification appears after the app learns that publication completed. iOS may delay that check until you reopen the app."
        ).font(.footnote)
        Link(
          "Open notification settings",
          destination: URL(string: UIApplication.openNotificationSettingsURLString)!)
      }
      StorageSettingsSections()
      NavigationLink("Help", value: SceneDestination.help)
    }.navigationTitle("Sync and storage")
  }
  private func preference(_ preference: SyncPreference) -> Binding<Bool> {
    Binding(
      get: { (model.workspace?.policy ?? model.guestPolicy)[keyPath: preference.keyPath] },
      set: { value in
        let account = model.activeAccountId
        Task { await model.setPreference(preference, value: value, accountId: account) }
      })
  }

}

struct HelpView: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    List {
      Section("Privacy and terms") {
        NavigationLink("Privacy", value: SceneDestination.legal("privacy"))
        NavigationLink("Terms", value: SceneDestination.legal("terms"))
      }
      Section("Your saved work") {
        Text(
          "Photos are copied into private app storage before they are marked saved. Public coordinates use your voyage's location precision. Only the server confirms publication."
        )
        Text(
          "Background uploads may wait for iOS or a permitted connection. Force-quitting stops background transfers until you reopen PinVoyager."
        )
        Text(
          "Unsynced drafts are not a device backup. Export them before removing the app. Exports may include exact photo locations."
        )
      }
      Section {
        Link("Contact support", destination: URL(string: "mailto:inquiries@bryanporter.com")!)
        ShareLink("Share diagnostic references", item: diagnostics)
      }
    }.navigationTitle("Help")
  }
  private var diagnostics: String {
    let references = model.workspace?.operations.compactMap(\.requestId) ?? []
    return
      "PinVoyager \(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1")\nRequest references: \(references.joined(separator: ", "))"
  }
}

struct ReportView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @Environment(\.dismiss) private var dismiss
  let target: String
  var subject: String?
  @State private var reason = "privacy_concern"
  @State private var details = ""
  @State private var queued = false
  @State private var operationId: UUID?
  private var received: Bool {
    model.workspace?.operations.first { $0.id == operationId }?.state == .applied
  }
  var body: some View {
    NavigationStack {
      Form {
        if let subject { Section { Text(subject).font(.headline) } }
        if queued {
          Label(
            received ? "Report received" : "Waiting to send",
            systemImage: received ? "checkmark.circle" : "clock")
          Text(
            received
              ? "The server confirmed receipt of your report."
              : "Saved on this device. Waiting for the server to confirm receipt.")
        } else {
          Picker("Reason", selection: $reason) {
            ForEach(
              ["harassment", "sexual_content", "violence", "privacy_concern", "spam", "other"],
              id: \.self
            ) { Text($0.replacingOccurrences(of: "_", with: " ").capitalized).tag($0) }
          }
          TextEditor(text: $details).frame(minHeight: 150).accessibilityLabel("Report details")
          Text("\(details.unicodeScalars.count) / 2,000 characters").font(.caption)
          if let id = model.activeAccountId {
            Button("Submit report") {
              Task {
                do {
                  operationId = try await model.repository.report(
                    accountId: id, target: target, reason: reason, details: details)
                  queued = true
                  await model.reloadWorkspace()
                  await model.sync()
                } catch { model.error = error.localizedDescription }
              }
            }.disabled(details.unicodeScalars.count > 2_000)
          } else {
            Button("Sign in to submit") {
              router.signInAfterDismiss = true
              dismiss()
            }
          }
        }
      }.navigationTitle("Report content").toolbar {
        Button(queued ? "Done" : "Cancel") { dismiss() }
      }
    }
  }
}
