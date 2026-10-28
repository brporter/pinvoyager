import PinVoyagerDomain
import SwiftUI

struct RootView: View {
  @Bindable var model: AppModel
  @Environment(\.horizontalSizeClass) private var sizeClass
  @State private var router = SceneRouter()
  @State private var window = WindowContext()
  @State private var restored = false
  @SceneStorage("routes.v1") private var savedRoutes = Data()
  @AppStorage("activeAccountId") private var savedAccount = ""

  var body: some View {
    VStack(spacing: 0) {
      connectionBanner
      Group {
        if sizeClass == .regular {
          NavigationSplitView {
            List(AppTab.allCases, id: \.self) { tab in
              Button {
                router.selectedTab = tab
              } label: {
                Label(tab.title, systemImage: tab.symbol)
              }
              .listRowBackground(
                router.selectedTab == tab ? Color.accentColor.opacity(0.12) : .clear)
            }.navigationTitle("PinVoyager")
          } detail: {
            destination(router.selectedTab)
          }
        } else {
          TabView(selection: $router.selectedTab) {
            ForEach(AppTab.allCases, id: \.self) { tab in
              Tab(tab.title, systemImage: tab.symbol, value: tab) {
                destination(tab)
              }.badge(tab == .sync ? model.workspace?.attentionCount ?? 0 : 0)
            }
          }
        }
      }
    }
    .sheet(
      isPresented: $router.showingSignIn,
      onDismiss: {
        if model.signedIn {
          Task { await router.resumePendingRoute(model: model) }
        } else {
          router.cancelAuthentication()
        }
      }
    ) { SignInView() }
    .sheet(
      isPresented: $router.showingNameSetup,
      onDismiss: {
        if model.workspace?.account.profile.displayName != nil {
          Task { await router.resumePendingRoute(model: model) }
        } else {
          router.cancelAuthentication()
        }
      }
    ) {
      NavigationStack { DisplayNameView(initialSetup: true) }
    }
    .sheet(isPresented: $router.showingCreation, onDismiss: router.creationDismissed) {
      VoyageEditor()
    }
    .alert(
      "Could not complete this action",
      isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
    ) {
      Button("OK") { model.error = nil }
    } message: {
      Text(model.error ?? "")
    }
    .onChange(of: router.restorationData(accountId: model.activeAccountId)) { _, value in
      if restored { savedRoutes = value }
    }
    .onChange(of: model.activeAccountId) { previous, next in
      savedAccount = next?.uuidString ?? ""
      router.accountChanged(from: previous, to: next)
    }
    .onAppear {
      guard !restored else { return }
      router.restore(savedRoutes, workspace: model.workspace)
      restored = true
      if router.pendingRoute != nil, model.workspace != nil {
        Task { await router.resumePendingRoute(model: model) }
      }
    }
    .onOpenURL { url in Task { await router.handle(url, model: model) } }
    .task { if !AppBootstrap.isUITesting { await model.activate() } }
    .environment(model)
    .environment(router)
    .environment(window)
    .background(WindowAccessor(context: window).frame(width: 0, height: 0))
  }

  @ViewBuilder private var connectionBanner: some View {
    if let banner {
      Button {
        router.selectedTab = .sync
      } label: {
        Label(banner, systemImage: model.offline ? "wifi.slash" : "info.circle")
          .font(.footnote).frame(maxWidth: .infinity, minHeight: 44).padding(.horizontal)
      }.background(.thinMaterial)
    }
  }

  private var banner: String? {
    if model.offline { return "Offline — changes stay on this iPhone" }
    if model.workspace?.account.access == .requiresSignIn { return "Sign in to resume syncing" }
    if case .cellular = model.network, model.workspace?.policy.wifiOnly == true {
      return "Waiting for Wi-Fi"
    }
    return nil
  }

  private func path(for tab: AppTab) -> Binding<[SceneDestination]> {
    switch tab {
    case .discover: $router.discoverPath
    case .voyages: $router.voyagePath
    case .sync: $router.syncPath
    case .account: $router.accountPath
    }
  }

  private func destination(_ tab: AppTab) -> some View {
    NavigationStack(path: path(for: tab)) {
      Group {
        switch tab {
        case .discover: DiscoverView()
        case .voyages: MyVoyagesView()
        case .sync: SyncView()
        case .account: AccountView()
        }
      }.navigationDestination(for: SceneDestination.self) { accountDestination($0) }
    }.id(tab)
  }
  @ViewBuilder private func accountDestination(_ destination: SceneDestination) -> some View {
    switch destination {
    case .voyage(let id): RemoteVoyageView(id: id)
    case .profile(let identifier): ProfileView(identifier: identifier)
    case .localVoyage(let id): LocalVoyageView(id: id)
    case .voyageSync(let id): SyncView(voyageId: id)
    case .legal(let id): BundledLegalView(id: id)
    case .conflict(let id):
      if let operation = model.workspace?.operations.first(where: {
        $0.id == id && $0.state == .conflict
      }) {
        ConflictResolutionView(operation: operation)
      } else {
        ContentUnavailableView("This conflict has been resolved", systemImage: "checkmark.circle")
      }
    case .downloads: ManageDownloadsView()
    case .export: ExportVoyagePickerView()
    case .exportVoyage(let id): ExportVoyageView(voyageId: id)
    case .preferences: SyncSettingsView()
    case .help: HelpView()
    case .ownProfile:
      if let id = model.activeAccountId { ProfileView(identifier: id.uuidString) }
    case .displayName: DisplayNameView()
    case .signInMethods: SignInMethodsView()
    case .blocked: BlockedAccountsView()
    case .deletion: DeleteAccountView()
    case .deletionRecovery: DeletionRecoveryView()
    }
  }

}
