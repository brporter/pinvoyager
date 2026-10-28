import Foundation
import PinVoyagerDomain

public struct ImportRecovery: Sendable {
  private let store: any WorkspaceStore
  private let preparer: PhotoPreparer
  public init(store: any WorkspaceStore, vault: FileVault) {
    self.store = store
    self.preparer = PhotoPreparer(vault: vault)
  }

  public func recover(accountId: UUID, maxBytes: Int64) async throws {
    let state = try await store.read(accountId: accountId)
    guard state.account.access != .locked else { return }
    let repository = LocalRepository(store: store)
    for pending in state.photos
    where !pending.deleted && pending.source == nil
      && [.acquiring, .needsSource].contains(pending.transfer)
    {
      do {
        let photo = try await preparer.recover(pending, accountId: accountId, maxBytes: maxBytes)
        try await repository.addPhoto(
          accountId: accountId, photo: photo,
          publicConsent: pending.importPublicConsent == true,
          expectedGeneration: state.account.generation)
      } catch WorkspaceError.locked {
        return
      } catch {
        try await repository.markImportMissing(
          accountId: accountId, photoId: pending.id,
          message: "Choose this photo again. \(error.localizedDescription)")
      }
    }
  }
}
