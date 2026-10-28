import Foundation
import Observation
import PinVoyagerData
import PinVoyagerDomain

struct PublicAccess: Hashable {
  let accountId: UUID?
  let generation: Int?
  let validation: UUID
  let offline: Bool
  let blocked: Set<UUID>
  let pendingBlocks: Bool
}

enum PublicationSignOutResult: Equatable, Sendable {
  case signedOut, waiting
  case published(Set<UUID>)
}

@MainActor @Observable
final class AppModel {
  let store: any WorkspaceStore
  let repository: LocalRepository
  let client: APIClient
  let credentials: any CredentialStore
  let authentication: NativeAuthentication
  let accountDeletion: AccountDeletionService
  var pendingDeletions: [PendingAccountDeletion] = []
  let vault: FileVault?
  var storage: StorageManager? {
    vault.map { StorageManager(store: store, vault: $0, cache: client.imageCache) }
  }
  var localDiscard: LocalDiscardService? {
    guard let vault else { return nil }
    let cancel = cancelIneligibleTransfers
    return LocalDiscardService(store: store, vault: vault) { account in await cancel?(account) }
  }
  let webOrigin: TrustedOrigin
  let devicePreferences: any DevicePreferences
  var guestPolicy = SyncPolicy()
  var workspace: Workspace?
  var identities: [LinkedIdentity] = []
  var capabilities: ClientCapabilities?
  var capabilityChecked = false
  var discover: [VoyageSummary] = []
  var discoverCursor: String?
  var discoverError: String?
  var loadingDiscover = false
  var error: String?
  var network: NetworkState = .wifi(constrained: false)
  var syncing = false
  private var activating = false
  private var publicValidation = UUID()
  var publicAccess: PublicAccess {
    PublicAccess(
      accountId: signedIn ? activeAccountId : nil,
      generation: signedIn ? workspace?.account.generation : nil,
      validation: publicValidation, offline: offline, blocked: workspace?.blockedAccounts ?? [],
      pendingBlocks: workspace?.operations.contains(where: {
        if case .block(_, true) = $0.kind { return ![.applied, .canceled].contains($0.state) }
        return false
      }) == true)
  }
  private var syncTask: Task<Void, Never>?
  private var syncRequested = false
  var downloadingVoyages: Set<UUID> = []
  var downloadingPhotos: Set<UUID> = []
  var activeAccountId: UUID?
  var coordinators: [UUID: SyncCoordinator] = [:]
  var makeCoordinator: ((UUID, ClientCapabilities) -> SyncCoordinator)?
  let publicationNotifier: PublicationNotifier
  var makeCamera: () -> CameraController = { CameraController() }
  var cancelPersistedTransfers: ((UUID) async -> Void)?
  var cancelIneligibleTransfers: (@MainActor @Sendable (UUID) async -> Void)?
  var signedIn: Bool { workspace?.account.access == .unlocked }
  var offline: Bool { network == .offline }
  var nativeSupported: Bool { capabilities?.supportsNativeSync == true }
  var nativeAuthenticationAvailable: Bool { capabilities?.supportsNativeAuthentication == true }
  var localVoyages: [LocalVoyage] {
    (workspace?.voyages ?? []).filter { !$0.deleted }.sorted {
      if ($0.serverId == nil) != ($1.serverId == nil) { return $0.serverId == nil }
      return $0.updatedAt > $1.updatedAt
    }
  }

  init(
    store: any WorkspaceStore, client: APIClient, credentials: any CredentialStore,
    vault: FileVault? = nil, webOrigin: TrustedOrigin? = nil,
    deletionRecovery: any DeletionRecoveryStore = KeychainDeletionRecoveryStore(),
    notifications: any PublicationNotifications = SystemPublicationNotifications(),
    devicePreferences: any DevicePreferences = UserDefaultsDevicePreferences()
  ) {
    self.store = store
    self.repository = LocalRepository(store: store)
    self.client = client
    self.credentials = credentials
    self.authentication = NativeAuthentication(client: client, credentials: credentials)
    self.accountDeletion = AccountDeletionService(client: client, recovery: deletionRecovery)
    self.publicationNotifier = PublicationNotifier(store: store, notifications: notifications)
    self.vault = vault
    self.webOrigin = webOrigin ?? client.origin
    self.devicePreferences = devicePreferences
  }

  func restore(accountId: UUID?) async {
    do {
      guestPolicy = try await devicePreferences.load()
      guestPolicy.notifyOnPublication = nil
    } catch {
      guestPolicy = SyncPolicy()
      self.error =
        "Saved device preferences could not be opened. Default network restrictions are in use."
    }
    guard let accountId else { return }
    do {
      let saved = try await store.read(accountId: accountId)
      guard saved.account.access != .locked else {
        workspace = nil
        return
      }
      workspace = saved
      activeAccountId = accountId
    } catch { self.error = error.localizedDescription }
  }

  func reloadWorkspace() async {
    guard let activeAccountId else {
      workspace = nil
      return
    }
    await restore(accountId: activeAccountId)
    discover.removeAll { workspace?.blockedAccounts.contains($0.ownerId) == true }
  }

  func checkCapabilities() async {
    do {
      capabilities = try await client.capabilities()
      capabilityChecked = true
    } catch { self.error = error.localizedDescription }
  }

  func activate() async {
    guard !activating, !offline else { return }
    activating = true
    publicValidation = UUID()
    discover = []
    defer { activating = false }
    await verifyAccount()
    await checkCapabilities()
    if nativeSupported {
      await reconcileBlocks()
      await sync()
    }
    await loadDiscover(refresh: true)
  }

  func reconcileBlocks() async {
    guard let accountId = activeAccountId, signedIn, !offline else { return }
    struct Blocks: Decodable, Sendable { let userIds: Set<UUID> }
    do {
      let server: Blocks = try await client.get(path: "/api/me/blocks", accountId: accountId)
      try await store.update(accountId: accountId) { state in
        guard state.account.access == .unlocked else { throw WorkspaceError.locked }
        var blocked = server.userIds
        for operation in state.operations where ![.applied, .canceled].contains(operation.state) {
          if case .block(let userId, let desired) = operation.kind,
            desired || state.blockedAccounts.contains(userId)
          {
            blocked.insert(userId)
          }
        }
        state.blockedAccounts = blocked
      }
      await reloadWorkspace()
      discover.removeAll { workspace?.blockedAccounts.contains($0.ownerId) == true }
    } catch { self.error = error.localizedDescription }
  }

  func verifyAccount() async {
    guard let id = activeAccountId, !offline,
      let initial = try? await store.read(accountId: id), initial.account.access != .locked
    else { return }
    let generation = initial.account.generation
    do {
      let account = try await client.me(id)
      guard account.user.id == id else { throw WorkspaceError.wrongAccount }
      try await store.update(accountId: id) { state in
        guard state.account.access != .locked, state.account.generation == generation else {
          throw WorkspaceError.locked
        }
        state.account.profile = account.user
        state.account.verifiedAt = Date()
        state.account.access = .unlocked
      }
      guard activeAccountId == id else { return }
      identities = account.identities
      await reloadWorkspace()
    } catch let error as APIError where error.status == 401 {
      do {
        try await store.update(accountId: id) { state in
          guard state.account.generation == generation, state.account.access != .locked else {
            throw WorkspaceError.locked
          }
          if state.account.access != .requiresSignIn { state.account.generation += 1 }
          state.account.access = .requiresSignIn
        }
        guard activeAccountId == id else { return }
        await cancelAccountTransfers(id)
        identities = []
        await reloadWorkspace()
      } catch WorkspaceError.locked { return } catch { self.error = error.localizedDescription }
    } catch WorkspaceError.locked { return } catch { self.error = error.localizedDescription }
  }

  func acceptSession(_ session: SessionExchange) async throws {
    let ids = try await store.accountIds()
    if ids.contains(session.user.id) {
      try await store.update(accountId: session.user.id) { state in
        state.account.generation += 1
        state.account.profile = session.user
        state.account.access = .unlocked
        state.account.verifiedAt = Date()
        for index in state.operations.indices
        where [.attention, .uncertain].contains(state.operations[index].state) {
          state.operations[index].nextAttempt = .distantPast
        }
      }
    } else {
      var initial = Workspace(account: LocalAccount(profile: session.user))
      initial.policy = guestPolicy
      initial.policy.notifyOnPublication = nil
      try await store.insert(initial)
    }
    await cancelAccountTransfers(session.user.id)
    if let old = activeAccountId, old != session.user.id {
      try await repository.lock(accountId: old)
      await cancelAccountTransfers(old)
    }
    publicValidation = UUID()
    discover = []
    activeAccountId = session.user.id
    identities = session.identities
    await reloadWorkspace()
    scheduleSync()
  }

  func saveDisplayName(_ value: String) async throws {
    guard let accountId = activeAccountId, signedIn, !offline else {
      throw AuthenticationError.signInRequired
    }
    let name = try DisplayName.normalize(value)
    _ = try await client.raw(
      method: "PATCH", path: "/api/me", body: APIJSON.encode(["display_name": name]),
      accountId: accountId)
    let account = try await client.me(accountId)
    guard activeAccountId == accountId, account.user.id == accountId,
      account.user.displayName != nil
    else { throw WorkspaceError.wrongAccount }
    try await store.update(accountId: accountId) { state in
      guard state.account.access == .unlocked else { throw WorkspaceError.locked }
      state.account.profile = account.user
      state.account.verifiedAt = Date()
    }
    identities = account.identities
    await reloadWorkspace()
  }

  func loadDiscover(refresh: Bool) async {
    guard !loadingDiscover else { return }
    guard !offline else {
      discover = []
      discoverError = "Public voyages need a connection to verify access."
      return
    }
    let access = publicAccess
    loadingDiscover = true
    defer { loadingDiscover = false }
    do {
      let page = try await client.voyages(
        before: refresh ? nil : discoverCursor, viewer: signedIn ? activeAccountId : nil)
      guard access == publicAccess, !Task.isCancelled else { return }
      let existing = refresh ? [] : discover
      var seen: Set<UUID> = []
      discover = (existing + page.voyages).filter {
        seen.insert($0.id).inserted && workspace?.blockedAccounts.contains($0.ownerId) != true
      }
      discoverCursor = page.nextBefore
      discoverError = nil
    } catch { if access == publicAccess { discoverError = error.localizedDescription } }
  }

  func refreshOwnerList() async {
    guard let accountId = activeAccountId, signedIn, !offline else { return }
    do {
      var cursor: String?
      var visited: Set<String> = []
      repeat {
        let page = try await client.voyages(before: cursor, owner: accountId)
        try await store.update(accountId: accountId) { state in
          guard state.account.access == .unlocked else { throw WorkspaceError.locked }
          for summary in page.voyages where summary.ownerId == accountId {
            let pending = state.operations.contains { operation in
              let localId = state.voyages.first(where: { $0.serverId == summary.id })?.id
              return operation.voyageId == localId
                && ![.applied, .canceled].contains(operation.state)
            }
            if let index = state.voyages.firstIndex(where: { $0.serverId == summary.id }) {
              if !pending {
                state.voyages[index].fields = VoyageFields(
                  title: summary.title, description: summary.description,
                  granularity: summary.granularity)
                state.voyages[index].confirmedPublished = summary.published
              }
            } else {
              var local = LocalVoyage(
                ownerId: accountId,
                fields: VoyageFields(
                  title: summary.title, description: summary.description,
                  granularity: summary.granularity))
              local.serverId = summary.id
              local.confirmedPublished = summary.published
              state.voyages.append(local)
            }
          }
        }
        cursor = page.nextBefore
        if let value = cursor, !visited.insert(value).inserted {
          throw URLError(.badServerResponse)
        }
      } while cursor != nil
      await reloadWorkspace()
    } catch { self.error = error.localizedDescription }
  }

  func refreshVoyage(_ localId: UUID) async {
    guard let accountId = activeAccountId, signedIn, !offline,
      let voyage = workspace?.voyages.first(where: { $0.id == localId }),
      let serverId = voyage.serverId
    else { return }
    do {
      let remote = try await client.voyage(serverId, accountId: accountId)
      guard remote.ownerId == accountId else { throw WorkspaceError.wrongAccount }
      let privatePhotos = try await client.ownerPhotos(voyageId: serverId, accountId: accountId)
      let metadata = Dictionary(
        privatePhotos.map { ($0.id, $0) }, uniquingKeysWith: { _, newest in newest })
      try await store.update(accountId: accountId) { state in
        guard state.account.access == .unlocked,
          let index = state.voyages.firstIndex(where: { $0.id == localId })
        else { throw WorkspaceError.locked }
        guard state.voyages[index].revision == voyage.revision else { return }
        let dirty = state.operations.contains {
          $0.voyageId == localId && ![.applied, .canceled].contains($0.state)
        }
        if !dirty && !state.workingCopies.contains(where: { $0.voyageId == localId }) {
          state.voyages[index].fields = remote.fields
          state.voyages[index].serverFields = remote.fields
          state.voyages[index].revision = remote.revision
          state.voyages[index].confirmedPublished = remote.published
        }
        var nextOrder = (state.photos(for: localId).map(\.order).max() ?? -1) + 1
        for photo in remote.photos {
          if let existing = state.photos.firstIndex(where: {
            $0.voyageId == localId && $0.serverId == photo.id
          }) {
            state.photos[existing].variant = photo.variantStatus
            if !dirty { state.photos[existing].order = photo.position }
          } else {
            var local = LocalPhoto(voyageId: localId, filename: "Photo \(photo.position)")
            local.serverId = photo.id
            local.width = photo.width
            local.height = photo.height
            local.capturedAt = photo.capturedAt
            local.coordinate = metadata[photo.id]?.coordinate
            local.variant = photo.variantStatus
            local.transfer = .accepted
            local.order = dirty ? nextOrder : photo.position
            nextOrder += 1
            state.photos.append(local)
          }
        }
      }
      if let vault, let revision = remote.revision {
        try await OwnerDownloads(client: client, store: store, vault: vault).purgeMissing(
          accountId: accountId,
          voyageId: localId, keeping: Set(remote.photos.map(\.id)), revision: revision)
      }
      await reloadWorkspace()
    } catch let error as APIError where error.status == 404 {
      if let vault, let revision = voyage.revision {
        try? await OwnerDownloads(client: client, store: store, vault: vault).purgeMissing(
          accountId: accountId,
          voyageId: localId, keeping: [], revision: revision)
      }
      await reloadWorkspace()
      self.error =
        "This website voyage is unavailable. Retained originals and local work have been kept."
    } catch { self.error = error.localizedDescription }
  }

  func downloadOwnerCopy(_ voyageId: UUID) async {
    guard let accountId = activeAccountId, signedIn, !offline, let vault else {
      error = "Connect and sign in before downloading a website copy."
      return
    }
    guard downloadingVoyages.insert(voyageId).inserted else { return }
    defer { downloadingVoyages.remove(voyageId) }
    await perform {
      try await OwnerDownloads(client: client, store: store, vault: vault).download(
        accountId: accountId, voyageId: voyageId)
      try await removeEligibleOriginals(accountId: accountId)
    }
  }

  func downloadFullCopy(_ photoId: UUID) async {
    guard let accountId = activeAccountId, signedIn, !offline, let vault,
      downloadingPhotos.insert(photoId).inserted
    else { return }
    defer { downloadingPhotos.remove(photoId) }
    await perform(synchronize: false) {
      try await OwnerDownloads(client: client, store: store, vault: vault).downloadFull(
        accountId: accountId, photoId: photoId)
      try await removeEligibleOriginals(accountId: accountId)
    }
  }

  private func removeEligibleOriginals(accountId: UUID) async throws {
    if try await store.read(accountId: accountId).policy.keepOriginals == false {
      try await storage?.removeUploadedOriginals(accountId: accountId)
    }
  }

  func removeOwnerCopy(_ voyageId: UUID) async {
    guard let accountId = activeAccountId, let vault else { return }
    await perform {
      try await OwnerDownloads(client: client, store: store, vault: vault).remove(
        accountId: accountId, voyageId: voyageId)
    }
  }

  func create(_ fields: VoyageFields) async throws -> UUID {
    guard let activeAccountId else { throw WorkspaceError.locked }
    let id = try await repository.createVoyage(accountId: activeAccountId, fields: fields)
    await reloadWorkspace()
    scheduleSync()
    return id
  }

  func edit(_ id: UUID, fields: VoyageFields) async throws {
    guard let activeAccountId else { throw WorkspaceError.locked }
    try await repository.editVoyage(accountId: activeAccountId, voyageId: id, fields: fields)
    await reloadWorkspace()
    scheduleSync()
  }

  func scheduleSync() {
    guard nativeSupported, activeAccountId != nil, !offline else { return }
    syncRequested = true
    guard syncTask == nil else { return }
    syncTask = Task { [weak self] in
      guard let self else { return }
      defer {
        syncTask = nil
        if syncRequested { scheduleSync() }
      }
      while syncRequested && !Task.isCancelled {
        syncRequested = false
        await syncPass()
      }
    }
  }

  func sync() async {
    scheduleSync()
    await syncTask?.value
  }

  private func syncPass() async {
    guard !syncing, let id = activeAccountId, let capabilities, capabilities.supportsNativeSync
    else { return }
    syncing = true
    defer { syncing = false }
    do {
      try await authentication.revokePendingCredentials()
      let coordinator =
        coordinators[id] ?? makeCoordinator?(id, capabilities)
        ?? SyncCoordinator(store: store, remote: NativeSyncRemote(client: client, accountId: id))
      coordinators[id] = coordinator
      try await coordinator.run(accountId: id, network: network)
      try await removeEligibleOriginals(accountId: id)
      try await publicationNotifier.notifyCompleted(accountId: id)
      await reloadWorkspace()
    } catch {
      self.error = error.localizedDescription
      await reloadWorkspace()
    }
  }

  func setSyncPolicy(_ policy: SyncPolicy) async {
    do {
      if let id = activeAccountId, let previous = workspace?.policy {
        try await repository.setPolicy(accountId: id, policy: policy)
        await reloadWorkspace()
        if previous.wifiOnly != policy.wifiOnly
          || previous.allowsConstrained != policy.allowsConstrained
          || previous.paused != policy.paused
        {
          await cancelAccountTransfers(id)
        }
        if !policy.keepOriginals { await removeUploadedOriginals() }
        await sync()
      } else {
        var saved = policy
        saved.notifyOnPublication = nil
        try await devicePreferences.save(saved)
        guestPolicy = saved
      }
    } catch { self.error = error.localizedDescription }
  }

  func setPreference(_ preference: SyncPreference, value: Bool, accountId: UUID?) async {
    guard activeAccountId == accountId else { return }
    do {
      if let accountId {
        let generation = workspace?.account.generation
        try await repository.setPreference(
          accountId: accountId, preference: preference, value: value)
        guard activeAccountId == accountId else { return }
        await reloadWorkspace()
        if workspace?.account.generation != generation { await cancelAccountTransfers(accountId) }
        if !value && preference == .keepOriginals { await removeUploadedOriginals() }
        await sync()
      } else {
        let previous = guestPolicy
        guestPolicy[keyPath: preference.keyPath] = value
        let saved = guestPolicy
        do { try await devicePreferences.save(saved) } catch {
          if guestPolicy == saved { guestPolicy = previous }
          throw error
        }
      }
    } catch { self.error = error.localizedDescription }
  }

  func removeUploadedOriginals() async {
    guard let account = activeAccountId else { return }
    do { try await storage?.removeUploadedOriginals(accountId: account) } catch {
      self.error = error.localizedDescription
    }
    await reloadWorkspace()
  }

  func setCellularException(batchId: UUID, allowed: Bool) async {
    guard let id = activeAccountId else { return }
    do {
      let generation = workspace?.account.generation
      try await repository.setCellularException(accountId: id, batchId: batchId, allowed: allowed)
      await reloadWorkspace()
      if workspace?.account.generation != generation { await cancelAccountTransfers(id) }
      await sync()
    } catch { self.error = error.localizedDescription }
  }

  func setPublicationNotifications(_ enabled: Bool) async {
    guard let id = activeAccountId else { return }
    do { try await publicationNotifier.setEnabled(enabled, accountId: id) } catch {
      self.error = error.localizedDescription
    }
    await reloadWorkspace()
  }

  func reloadDeletionRecovery() async {
    do {
      pendingDeletions = try await accountDeletion.recovery.all().sorted {
        $0.id.uuidString < $1.id.uuidString
      }
    } catch { self.error = error.localizedDescription }
  }

  func refreshDeletion(accountId: UUID) async {
    do {
      let record = try await accountDeletion.refresh(accountId: accountId)
      if [.deleting, .complete].contains(record.status), activeAccountId == accountId {
        await signOut()
      } else if record.status == .prepared, activeAccountId == accountId {
        try await store.update(accountId: accountId) { state in
          if state.account.access == .locked { state.account.access = .requiresSignIn }
        }
        await reloadWorkspace()
      }
    } catch { self.error = error.localizedDescription }
    await reloadDeletionRecovery()
  }

  func confirmAccountDeletion() async {
    guard let accountId = activeAccountId, signedIn, !offline else { return }
    do {
      try await repository.lock(accountId: accountId, pauseSync: true)
      syncRequested = false
      syncTask?.cancel()
      workspace = nil
      identities = []
      await cancelAccountTransfers(accountId)
      _ = try await accountDeletion.confirm(accountId: accountId)
      await signOut()
      await reloadDeletionRecovery()
    } catch {
      self.error = error.localizedDescription
      await refreshDeletion(accountId: accountId)
    }
  }

  private func cancelAccountTransfers(_ accountId: UUID) async {
    if let cancelPersistedTransfers {
      await cancelPersistedTransfers(accountId)
    } else {
      await coordinators[accountId]?.cancelTransfers(accountId: accountId)
    }
    coordinators.removeValue(forKey: accountId)
  }

  func cancelPublicationAndSignOut(intentIds: Set<UUID>) async -> PublicationSignOutResult {
    guard let accountId = activeAccountId, signedIn, !offline, nativeSupported, !intentIds.isEmpty
    else {
      error = "Connect and sign in to confirm publication cancellation before signing out."
      return .waiting
    }
    do {
      let state = try await store.read(accountId: accountId)
      let intents = state.publicationIntents.filter { intentIds.contains($0.id) }
      guard intents.count == intentIds.count else { throw WorkspaceError.missingVoyage }
      for intent in intents where state.publicationMayProceed(intent) || intent.status == .queued {
        try await repository.cancelPublication(accountId: accountId, intentId: intent.id)
      }
      await reloadWorkspace()
      await sync()
      guard activeAccountId == accountId else { return .waiting }
      let latest = try await store.read(accountId: accountId)
      let reviewed = latest.publicationIntents.filter { intentIds.contains($0.id) }
      let published = Set(reviewed.filter { $0.status == .published }.map(\.voyageId))
      if !published.isEmpty { return .published(published) }
      guard reviewed.count == intentIds.count,
        reviewed.allSatisfy({
          [.canceled, .expired, .needsReview, .failed].contains($0.status)
            && !$0.cancellationPending
        })
      else {
        error =
          "Cancellation is not confirmed. You remain signed in. Check Sync or explicitly leave publication scheduled."
        return .waiting
      }
      await signOut()
      return activeAccountId == nil ? .signedOut : .waiting
    } catch {
      self.error = error.localizedDescription
      return .waiting
    }
  }

  func unpublishAndSignOut(voyageIds: Set<UUID>) async -> Bool {
    guard let accountId = activeAccountId, signedIn, !offline, nativeSupported, !voyageIds.isEmpty
    else {
      error = "Connect and sign in to make the voyage private before signing out."
      return false
    }
    do {
      for id in voyageIds { try await repository.unpublish(accountId: accountId, voyageId: id) }
      await reloadWorkspace()
      await sync()
      guard activeAccountId == accountId else { return false }
      let latest = try await store.read(accountId: accountId)
      guard
        voyageIds.allSatisfy({ id in
          latest.voyages.contains { $0.id == id && !$0.confirmedPublished }
        }),
        !latest.publicationIntents.contains(where: { voyageIds.contains($0.voyageId) && $0.isArmed }
        )
      else {
        error =
          "The website has not confirmed that the voyage is private. You remain signed in. Check Sync before trying again."
        return false
      }
      await signOut()
      return activeAccountId == nil
    } catch {
      self.error = error.localizedDescription
      return false
    }
  }

  func signOut(purge: Bool = false) async {
    guard let id = activeAccountId else { return }
    do {
      try await repository.lock(accountId: id)
      syncRequested = false
      syncTask?.cancel()
      workspace = nil
      identities = []
      activeAccountId = nil
      discover = []
      publicValidation = UUID()
      await cancelAccountTransfers(id)
      if var credential = try await credentials.read(accountId: id) {
        credential.revocationOnly = true
        try await credentials.save(credential)
      }
      if purge {
        try await store.remove(accountId: id)
        try await vault?.removeAccount(id)
      }
      if !offline { try await authentication.revokePendingCredentials() }
    } catch { self.error = error.localizedDescription }
  }

  func changeNetwork(_ network: NetworkState) {
    self.network = network
    if network == .offline {
      discover = []
      discoverError = "Public voyages are not available offline."
    }
  }

  func perform(synchronize: Bool = true, _ command: () async throws -> Void) async {
    do {
      try await command()
      if let account = activeAccountId { await cancelIneligibleTransfers?(account) }
      await reloadWorkspace()
      if synchronize { scheduleSync() }
    } catch { self.error = error.localizedDescription }
  }
}
