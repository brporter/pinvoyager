import AuthenticationServices
import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct AccountView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @State private var signOut = false
  @State private var signingOut = false
  @State private var publishedBeforeSignOut: Set<UUID> = []
  private var armedPublicationIds: Set<UUID> {
    guard let state = model.workspace else { return [] }
    return Set(state.publicationIntents.filter { state.publicationMayProceed($0) }.map(\.id))
  }
  private func beginSignOut(_ action: @escaping @MainActor () async -> Void) {
    guard !signingOut else { return }
    signingOut = true
    Task {
      defer { signingOut = false }
      await action()
    }
  }
  var body: some View {
    Form {
      if let account = model.workspace?.account, account.access == .unlocked {
        if !publishedBeforeSignOut.isEmpty {
          Section("Publication completed before cancellation") {
            Text("The voyage is public. Choose whether to make it private before signing out.")
            Button("Unpublish now and sign out") {
              beginSignOut {
                _ = await model.unpublishAndSignOut(voyageIds: publishedBeforeSignOut)
              }
            }.disabled(signingOut || model.offline)
            Button("Leave voyage public and sign out") { beginSignOut { await model.signOut() } }
              .disabled(signingOut)
            Button("Keep account open") { publishedBeforeSignOut = [] }.disabled(signingOut)
          }
        }
        if signingOut { ProgressView("Confirming sign out") }
        Section {
          Image("HorizonPin").resizable().scaledToFit().frame(width: 64, height: 64)
            .accessibilityHidden(true)
          Text(account.profile.displayName ?? "Choose your display name").font(.title2.bold())
          if model.offline {
            Label("Saved on this iPhone", systemImage: "iphone")
            Text("Connect to verify your account").font(.footnote)
          } else {
            NavigationLink("View public profile", value: SceneDestination.ownProfile)
            NavigationLink("Change display name", value: SceneDestination.displayName)
            NavigationLink("Sign-in methods", value: SceneDestination.signInMethods)
            NavigationLink("Blocked accounts", value: SceneDestination.blocked)
          }
          LabeledContent("Last verified") { Text(account.verifiedAt, style: .date) }
        }
        Section {
          if account.profile.isSiteAdmin == true {
            Link("Administration website", destination: model.webOrigin.url(path: ["admin"]))
          }
          Button("Sign out", role: .destructive) { signOut = true }.disabled(signingOut)
          NavigationLink("Delete account", value: SceneDestination.deletion).disabled(
            model.offline)
        }
      } else {
        Section {
          ContentUnavailableView {
            Label("Sign in to your account", systemImage: "person.crop.circle")
          } description: {
            Text("Save voyages, manage your photographs and choose when to publish.")
          } actions: {
            Button("Sign in") { router.showingSignIn = true }.buttonStyle(.borderedProminent)
          }
        }
      }
      if !model.pendingDeletions.isEmpty {
        Section {
          NavigationLink("Account deletion status", value: SceneDestination.deletionRecovery)
        }
      }
      Section {
        NavigationLink("Sync and storage", value: SceneDestination.preferences)
        NavigationLink("Privacy, terms and help", value: SceneDestination.help)
      }
      Section {
        Text(
          "PinVoyager \(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1") (\(Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"))"
        ).font(.footnote)
      }
    }
    .navigationTitle("Account")
    .task { if !AppBootstrap.isUITesting { await model.reloadDeletionRecovery() } }
    .confirmationDialog(
      "Sign out of this account?", isPresented: $signOut, titleVisibility: .visible
    ) {
      if !armedPublicationIds.isEmpty {
        if !model.offline {
          Button("Cancel publication and sign out") {
            let ids = armedPublicationIds
            beginSignOut {
              if case .published(let voyages) = await model.cancelPublicationAndSignOut(
                intentIds: ids)
              {
                publishedBeforeSignOut = voyages
              }
            }
          }.disabled(!model.nativeSupported)
        }
        Button("Leave publication scheduled and sign out") {
          beginSignOut { await model.signOut() }
        }
        Button("Delete local drafts and leave publication scheduled", role: .destructive) {
          beginSignOut { await model.signOut(purge: true) }
        }
      } else {
        Button("Keep drafts on this device and sign out") { beginSignOut { await model.signOut() } }
        Button("Delete local drafts and sign out", role: .destructive) {
          beginSignOut { await model.signOut(purge: true) }
        }
      }
      Button("Cancel", role: .cancel) {}
    } message: {
      Text(
        "Kept drafts remain locked until this account signs in again. A server-accepted publication may still finish. Offline sign-out cannot revoke the server session until you reconnect."
      )
    }
    .onChange(of: model.activeAccountId) { _, _ in publishedBeforeSignOut = [] }
  }
}

struct SignInView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @Environment(\.dismiss) private var dismiss
  var linking = false
  @State private var ephemeral = false
  private var busy: Bool { presentation?.busy == true }
  private var error: String? { presentation?.error }
  @Environment(WindowContext.self) private var window
  @State private var presentation: AuthenticationPresentation?
  var body: some View {
    NavigationStack {
      Form {
        Section {
          Image("HorizonPin").resizable().scaledToFit().frame(width: 80, height: 80).frame(
            maxWidth: .infinity
          ).accessibilityHidden(true)
          Text(linking ? "Link a sign-in method" : "Sign in to PinVoyager").font(.title2.bold())
          if model.offline {
            Text("Connect to sign in")
          } else if model.capabilityChecked && !model.nativeAuthenticationAvailable {
            Text(
              "Native sign-in is not available on this server yet. Public voyages remain available in Discover."
            )
          }
          ForEach(
            model.capabilities?.providers.filter { ["apple", "google", "microsoft"].contains($0) }
              ?? [], id: \.self
          ) { provider in
            Button("Continue with \(provider.capitalized)") { signIn(provider) }.disabled(
              busy || model.offline || !model.nativeAuthenticationAvailable
            ).frame(minHeight: 44)
          }
          if busy { ProgressView("Finishing sign-in") }
          Toggle("Use another account", isOn: $ephemeral).disabled(busy)
          if let error { ErrorNotice(message: error) }
          Button("Refresh sign-in options") { Task { await model.checkCapabilities() } }.disabled(
            busy || model.offline)
        }
        Section {
          Link("Terms", destination: model.webOrigin.url(path: ["terms"]))
          Link("Privacy", destination: model.webOrigin.url(path: ["privacy"]))
        }
      }.navigationTitle(linking ? "Sign-in methods" : "Sign in").toolbar {
        Button("Cancel") {
          if !linking { router.cancelAuthentication() }
          dismiss()
        }.disabled(busy)
      }
    }.task { await model.checkCapabilities() }.interactiveDismissDisabled(busy)
  }
  private func signIn(_ provider: String) {
    let presentation =
      self.presentation
      ?? AuthenticationPresentation(
        authentication: model.authentication,
        browser: BrowserAuthentication(anchor: { window.anchor() }))
    self.presentation = presentation
    let account = model.activeAccountId
    let generation = model.workspace?.account.generation
    Task {
      if await presentation.signIn(
        provider: provider, linkingAccountId: linking ? account : nil,
        ephemeral: ephemeral,
        accept: { session in
          guard model.activeAccountId == account, model.workspace?.account.generation == generation
          else { throw AuthenticationError.canceled }
          try await model.acceptSession(session)
        })
      {
        dismiss()
      }
    }
  }

}

struct DisplayNameView: View {
  var initialSetup = false
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var value = ""
  @State private var error: String?
  @State private var saving = false
  var body: some View {
    Form {
      Section {
        TextField("Display name", text: $value).textInputAutocapitalization(.never)
          .autocorrectionDisabled()
        Text(
          "Use 3–30 letters, digits or hyphens. Start with a letter and end with a letter or digit."
        ).font(.footnote)
        Text("Previous names continue to resolve to your profile.").font(.footnote)
        if let name = try? DisplayName.normalize(value) {
          Text(model.webOrigin.url(path: ["u", name]).absoluteString).font(.caption).textSelection(
            .enabled)
        }
        if let error { ErrorNotice(message: error) }
        Button(initialSetup ? "Continue" : "Save") { save() }.disabled(model.offline || saving)
      }
    }.navigationTitle(initialSetup ? "Choose your display name" : "Display name")
      .toolbar { if initialSetup { Button("Continue browsing") { dismiss() } } }
      .onAppear {
        value = model.workspace?.account.profile.displayName ?? ""
      }
  }
  private func save() {
    saving = true
    Task {
      do {
        try await model.saveDisplayName(value)
        dismiss()
      } catch let error as APIError where error.status == 409 {
        self.error = "That name is unavailable."
      } catch { self.error = error.localizedDescription }
      saving = false
    }
  }
}

struct SignInMethodsView: View {
  @Environment(AppModel.self) private var model
  @State private var linking = false
  var body: some View {
    List(["apple", "google", "microsoft"], id: \.self) { provider in
      HStack {
        Text(provider.capitalized)
        Spacer()
        if model.identities.contains(where: { $0.provider == provider }) {
          Label("Linked", systemImage: "checkmark")
        } else {
          Button("Link") { linking = true }.disabled(
            model.offline || !model.nativeAuthenticationAvailable)
        }
      }
    }.navigationTitle("Sign-in methods").sheet(isPresented: $linking) { SignInView(linking: true) }
  }
}

struct BlockedAccountsView: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    List {
      if model.workspace?.blockedAccounts.isEmpty != false { Text("No blocked travelers") }
      ForEach(
        Array(model.workspace?.blockedAccounts ?? []).sorted { $0.uuidString < $1.uuidString },
        id: \.self
      ) { id in
        HStack {
          Text(id.uuidString).font(.caption)
          Spacer()
          Button("Unblock") {
            if let accountId = model.activeAccountId {
              Task {
                await model.perform {
                  try await model.repository.setBlocked(
                    accountId: accountId, userId: id, blocked: false)
                }
                await model.sync()
              }
            }
          }.disabled(model.offline)
        }
      }
    }.navigationTitle("Blocked accounts")
  }
}

struct DeleteAccountView: View {
  @Environment(AppModel.self) private var model
  @Environment(WindowContext.self) private var window
  @State private var presentation: AuthenticationPresentation?
  @State private var performing = false
  private var busy: Bool { performing || presentation?.busy == true }
  @State private var error: String?
  @State private var confirming = false
  @State private var prepared: PendingAccountDeletion?
  var body: some View {
    Form {
      Section {
        Text(
          "Account deletion removes your website voyages, photos, sessions and linked identities. Former display names remain reserved. File cleanup continues after access is removed."
        )
        Text(
          "Export unsynchronized local work first. Local drafts remain locked on this device unless you remove them separately."
        )
        NavigationLink("Export local work", value: SceneDestination.export)
      }
      Section {
        if let prepared, [.confirming, .deleting].contains(prepared.status) {
          Text(prepared.status.message)
          NavigationLink("Check pending deletion", value: SceneDestination.deletionRecovery)
        } else if let prepared, prepared.status == .preparing, prepared.expiresAt > Date() {
          Text(prepared.status.message)
          Button("Retry preparation") { Task { await retryPreparation() } }.disabled(
            busy || model.offline)
          Button("Check preparation status") { Task { await checkPreparation() } }.disabled(
            busy || model.offline)
        } else if let prepared, prepared.expiresAt > Date(), prepared.status == .prepared {
          Text("Your account is ready for deletion. It has not been deleted yet.")
          Button("Confirm account deletion", role: .destructive) { confirming = true }.disabled(
            busy || model.offline)
        } else {
          if model.identities.contains(where: { $0.provider == "apple" }) {
            Text(
              "Verify your linked Apple account so PinVoyager can revoke its authorization before deletion."
            )
          } else {
            Text("Verify a linked sign-in method before confirming deletion.")
          }
          ForEach(providers, id: \.self) { provider in
            Button("Verify with \(provider.capitalized)") { authenticate(provider) }.disabled(
              busy || model.offline)
          }
        }
        if busy { ProgressView("Verifying account deletion") }
        if let error = error ?? presentation?.error { ErrorNotice(message: error) }
        NavigationLink("Help and contact", value: SceneDestination.help)
      }
    }.navigationTitle("Delete account")
      .confirmationDialog(
        "Permanently delete this account?", isPresented: $confirming, titleVisibility: .visible
      ) {
        Button("Delete my account", role: .destructive) {
          performing = true
          Task {
            await model.confirmAccountDeletion()
            performing = false
          }
        }
      } message: {
        Text("Your public voyages will be removed and all sessions revoked. This cannot be undone.")
      }
      .task {
        if let accountId = model.activeAccountId {
          prepared = try? await model.accountDeletion.recovery.read(accountId: accountId)
        }
      }.interactiveDismissDisabled(busy)
  }
  private func retryPreparation() async {
    guard let accountId = model.activeAccountId, !busy else { return }
    performing = true
    defer { performing = false }
    do {
      prepared = try await model.accountDeletion.retryPreparation(accountId: accountId)
      await model.reloadDeletionRecovery()
    } catch { self.error = error.localizedDescription }
  }
  private func checkPreparation() async {
    guard let accountId = model.activeAccountId, !busy else { return }
    performing = true
    defer { performing = false }
    await model.refreshDeletion(accountId: accountId)
    prepared = try? await model.accountDeletion.recovery.read(accountId: accountId)
  }
  private var providers: [String] {
    let linked = model.identities.map(\.provider)
    return linked.contains("apple") ? ["apple"] : linked
  }
  private func authenticate(_ provider: String) {
    guard let account = model.activeAccountId else { return }
    let presentation =
      self.presentation
      ?? AuthenticationPresentation(
        authentication: model.authentication,
        browser: BrowserAuthentication(anchor: { window.anchor() }))
    self.presentation = presentation
    error = nil
    let generation = model.workspace?.account.generation
    Task {
      _ = await presentation.reauthenticate(provider: provider, accountId: account) { proof in
        guard model.activeAccountId == account, model.workspace?.account.generation == generation
        else { throw AuthenticationError.canceled }
        prepared = try await model.accountDeletion.prepare(
          accountId: account, proofToken: proof.proofToken)
        await model.reloadDeletionRecovery()
      }
    }
  }

}

struct DeletionRecoveryView: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    List {
      ForEach(model.pendingDeletions) { record in
        Section {
          LabeledContent("Deletion request", value: record.id.uuidString).font(.caption)
          Text(record.status.message)
          Button("Check deletion status") {
            Task { await model.refreshDeletion(accountId: record.accountId) }
          }.disabled(model.offline)
          if [.complete, .expired].contains(record.status) {
            Button("Remove this status record", role: .destructive) {
              Task {
                do {
                  try await model.accountDeletion.recovery.remove(accountId: record.accountId)
                  await model.reloadDeletionRecovery()
                } catch { model.error = error.localizedDescription }
              }
            }
          }
        }
      }
      Text(
        "Status records contain a private recovery credential and remain on this device. The server retains the minimal outcome for 30 days."
      ).font(.footnote)
    }.navigationTitle("Account deletion status")
  }
}
extension AccountDeletionStatus {
  var message: String {
    switch self {
    case .preparing:
      "Checking whether preparation reached the server. Your account has not been confirmed for deletion."
    case .prepared: "Prepared. Your account remains active until you confirm deletion."
    case .confirming: "Checking whether account deletion completed."
    case .deleting: "Public access has been removed. File cleanup continues."
    case .complete: "Account deletion is complete."
    case .expired: "Preparation expired. Authenticate again to prepare a new request."
    }
  }
}
