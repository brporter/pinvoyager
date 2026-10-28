import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

@main
struct PinVoyagerApp: App {
  @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
  @State private var bootstrap = AppBootstrap.shared
  @Environment(\.scenePhase) private var phase

  var body: some Scene {
    WindowGroup {
      Group {
        if let model = bootstrap.model {
          RootView(model: model)
        } else if let error = bootstrap.error {
          ContentUnavailableView {
            Label(
              "Saved work could not be opened", systemImage: "externaldrive.badge.exclamationmark")
          } description: {
            Text(error)
          } actions: {
            Button("Try again") { Task { await bootstrap.start() } }
            ShareLink("Export recoverable local files", item: AppBootstrap.directory)
          }
        } else {
          ProgressView("Opening saved voyages")
        }
      }
      .task { await bootstrap.start() }
      .onChange(of: phase) { _, phase in
        guard !AppBootstrap.isUITesting else { return }
        if phase == .active {
          Task {
            await bootstrap.model?.activate()
          }
        }
        if phase == .background { delegate.schedulePreparation() }
      }
    }
  }
}

@MainActor @Observable
final class AppBootstrap {
  static let shared = AppBootstrap()
  var model: AppModel?
  var error: String?
  private let makeStore: @Sendable (URL?) async throws -> any WorkspaceStore
  private var startup: Task<Void, Never>?
  private let environment: AppEnvironment?
  init(
    environment: AppEnvironment? = nil,
    makeStore: @escaping @Sendable (URL?) async throws -> any WorkspaceStore = { url in
      try await Task.detached { try CoreDataWorkspaceStore(url: url) }.value
    }
  ) {
    self.makeStore = makeStore
    self.environment = environment
  }
  private var monitor: NetworkObserver?
  nonisolated static var directory: URL {
    URL.applicationSupportDirectory.appendingPathComponent("PinVoyager", isDirectory: true)
  }

  func start() async {
    guard model == nil else { return }
    if let startup {
      await startup.value
      return
    }
    let task = Task { await open() }
    startup = task
    await task.value
    startup = nil
  }

  private func open() async {
    do {
      let environment = try environment ?? AppEnvironment.system(testing: Self.isUITesting)
      let testing = environment.testing
      let store = try await makeStore(
        testing ? nil : environment.directory.appendingPathComponent("workspace.sqlite"))
      #if DEBUG
        let apiOrigin = try TrustedOrigin(environment.apiURL, allowsLocalHTTP: true)
      #else
        let apiOrigin = try TrustedOrigin(environment.apiURL)
      #endif
      let credentials = environment.credentials
      let imageCache = try TemporaryImageCache(directory: environment.cacheDirectory)
      let client = APIClient(
        origin: apiOrigin, transport: environment.transport, credentials: credentials,
        imageCache: imageCache)
      let vault = try FileVault(root: environment.directory.appendingPathComponent("files"))
      let model = AppModel(
        store: store, client: client, credentials: credentials, vault: vault,
        webOrigin: try TrustedOrigin(environment.webURL),
        deletionRecovery: environment.deletionRecovery,
        devicePreferences: environment.preferences)
      #if DEBUG
        if testing { try await DevelopmentFixtures.seed(model) }
      #endif
      if !testing {
        let id = environment.activeAccountId
        await model.restore(accountId: id)
        if let id {
          try await ImportRecovery(store: store, vault: vault).recover(
            accountId: id, maxBytes: 25 * 1_024 * 1_024)
          await model.reloadWorkspace()
        }
        monitor = NetworkObserver { [weak model] network in
          model?.changeNetwork(network)
          if network != .offline { Task { await model?.sync() } }
        }
        try await configureBackground(
          model: model, vault: vault, credentials: credentials, origin: apiOrigin,
          driver: environment.transfers)
      }
      self.model = model
      self.error = nil
    } catch { self.error = error.localizedDescription }
  }

  static var isUITesting: Bool {
    #if DEBUG
      ProcessInfo.processInfo.arguments.contains("--ui-testing")
        || ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    #else
      false
    #endif
  }

  func configureBackground(
    model: AppModel, vault: FileVault, credentials: any CredentialStore, origin: TrustedOrigin,
    driver: SystemTransferDriver = .shared
  ) async throws {
    let recovery = BackgroundTransferRecovery(
      store: model.store, credentials: credentials, driver: driver)
    driver.onCompletion = { [weak model] descriptor in
      try await recovery.completed(descriptor)
      Task { @MainActor in model?.scheduleSync() }
    }
    model.cancelPersistedTransfers = { accountId in await recovery.cancel(accountId: accountId) }
    model.cancelIneligibleTransfers = { accountId in
      await recovery.cancelIneligible(accountId: accountId)
    }
    let store = model.store
    let client = model.client
    model.makeCoordinator = { accountId, capabilities in
      let uploads = BackgroundUploadService(
        store: store, vault: vault, credentials: credentials, driver: driver,
        origin: origin, maxBytes: capabilities.maxUploadBytes)
      return SyncCoordinator(
        store: store, remote: NativeSyncRemote(client: client, accountId: accountId),
        uploads: uploads)
    }
    try await recovery.restore()
    if let account = model.activeAccountId {
      try await model.localDiscard?.finishPendingRemovals(accountId: account)
    }
  }
}
