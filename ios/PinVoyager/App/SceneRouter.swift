import Foundation
import Observation
import PinVoyagerDomain

enum SceneDestination: Codable, Hashable {
  case voyage(UUID)
  case profile(String)
  case localVoyage(UUID)
  case voyageSync(UUID)
  case preferences, help, ownProfile, displayName, signInMethods, blocked, deletion,
    deletionRecovery, downloads, export
  case exportVoyage(UUID)
  case conflict(UUID)
  case legal(String)
}

enum OwnerAction: Equatable {
  case edit(UUID)
  case addPhotos(UUID)
}

@MainActor @Observable
final class SceneRouter {
  var selectedTab: AppTab = .discover
  var discoverPath: [SceneDestination] = []
  var voyagePath: [SceneDestination] = []
  var syncPath: [SceneDestination] = []
  var accountPath: [SceneDestination] = []
  var showingSignIn = false
  var signInAfterDismiss = false
  var showingNameSetup = false
  var showingCreation = false
  var pendingRoute: AppRoute?
  var deferredRoute: AppRoute?
  var ownerAction: OwnerAction?
  var beforeNavigation: (() async throws -> Void)?
  private var createdVoyageForImport: UUID?

  private struct Saved: Codable {
    let version: Int
    let accountId: UUID?
    let tab: AppTab
    let discover: [SceneDestination]
    let voyages: [SceneDestination]
    let sync: [SceneDestination]
    let account: [SceneDestination]
    let pending: AppRoute?
    let deferred: AppRoute?
  }

  func restorationData(accountId: UUID?) -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    return
      (try? encoder.encode(
        Saved(
          version: 1, accountId: accountId, tab: selectedTab,
          discover: discoverPath, voyages: voyagePath, sync: syncPath,
          account: accountPath, pending: pendingRoute, deferred: deferredRoute))) ?? Data()
  }

  func restore(_ data: Data, workspace: Workspace?) {
    guard let saved = try? JSONDecoder().decode(Saved.self, from: data), saved.version == 1 else {
      return
    }
    selectedTab = saved.tab
    discoverPath = saved.discover.filter {
      switch $0 {
      case .voyage, .profile: true
      default: false
      }
    }
    let sameAccount =
      saved.accountId == workspace?.account.id && workspace?.account.access != .locked
    let ids = Set(workspace?.voyages.filter { !$0.deleted }.map(\.id) ?? [])
    func available(_ route: SceneDestination) -> Bool {
      switch route {
      case .localVoyage(let id), .voyageSync(let id), .exportVoyage(let id): ids.contains(id)
      default: true
      }
    }
    voyagePath = sameAccount ? saved.voyages.filter(available) : []
    syncPath = sameAccount ? saved.sync.filter(available) : []
    accountPath = sameAccount ? saved.account : []
    pendingRoute = sameAccount ? saved.pending : nil
    deferredRoute = sameAccount ? saved.deferred : nil
    showingSignIn = pendingRoute != nil && workspace == nil
  }

  func accountChanged(from previous: UUID?, to next: UUID?) {
    guard previous != nil, previous != next else { return }
    discoverPath = []
    voyagePath = []
    syncPath = []
    accountPath = []
    ownerAction = nil
    createdVoyageForImport = nil
    showingCreation = false
    beforeNavigation = nil
    deferredRoute = nil
    cancelAuthentication()
  }

  func requestCreation(hasWorkspace: Bool) {
    if hasWorkspace {
      showingCreation = true
    } else {
      pendingRoute = .createVoyage
      showingSignIn = true
    }
  }

  func creationCompleted(voyageId: UUID, addPhotos: Bool) {
    selectedTab = .voyages
    createdVoyageForImport = addPhotos ? voyageId : nil
  }

  func creationDismissed() {
    guard let id = createdVoyageForImport else { return }
    createdVoyageForImport = nil
    voyagePath.append(.localVoyage(id))
    ownerAction = .addPhotos(id)
  }

  func sheetDismissed() {
    guard signInAfterDismiss else { return }
    signInAfterDismiss = false
    showingSignIn = true
  }

  func cancelAuthentication() {
    pendingRoute = nil
    signInAfterDismiss = false
    showingSignIn = false
    showingNameSetup = false
  }

  func resumePendingRoute(model: AppModel) async {
    guard model.workspace != nil else { return }
    showingSignIn = false
    guard model.workspace?.account.profile.displayName != nil else {
      showingNameSetup = true
      return
    }
    showingNameSetup = false
    guard let route = pendingRoute else { return }
    pendingRoute = nil
    await navigate(route, model: model)
  }

  func handle(_ url: URL, model: AppModel) async {
    guard let route = AppRoute.parse(url, origin: model.webOrigin) else { return }
    deferredRoute = route
    await retryNavigation(model: model)
  }

  func retryNavigation(model: AppModel) async {
    guard let route = deferredRoute else { return }
    do {
      try await beforeNavigation?()
      beforeNavigation = nil
      deferredRoute = nil
      await navigate(route, model: model)
    } catch { model.error = error.localizedDescription }
  }

  private func navigate(_ route: AppRoute, model: AppModel) async {
    switch route {
    case .account: selectedTab = .account
    case .createVoyage: requestCreation(hasWorkspace: model.workspace != nil)
    case .editVoyage(let id), .addPhotos(let id):
      guard let accountId = model.activeAccountId else {
        pendingRoute = route
        showingSignIn = true
        return
      }
      do {
        let voyage = try await model.client.voyage(id, accountId: accountId)
        guard model.activeAccountId == accountId, voyage.ownerId == accountId else {
          throw WorkspaceError.missingVoyage
        }
        await model.refreshOwnerList()
        guard
          let localId = model.workspace?.voyages.first(where: { $0.serverId == id && !$0.deleted })?
            .id
        else {
          throw WorkspaceError.missingVoyage
        }
        selectedTab = .voyages
        voyagePath.append(.localVoyage(localId))
        if case .editVoyage = route {
          ownerAction = .edit(localId)
        } else {
          ownerAction = .addPhotos(localId)
        }
      } catch { model.error = error.localizedDescription }
    case .voyage(let id):
      selectedTab = .discover
      discoverPath.append(.voyage(id))
    case .profile(let name):
      selectedTab = .discover
      discoverPath.append(.profile(name))
    }
  }
}
