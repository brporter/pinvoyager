import Foundation
import Testing

@testable import PinVoyagerDomain

@Test func syncMilestonesRequireAcceptedAndProcessedPhotosAndConfirmedPublication() throws {
  var state = Workspace(
    account: LocalAccount(profile: UserProfile(id: UUID(), displayName: "traveler")))
  var voyage = LocalVoyage(
    ownerId: state.account.id,
    fields: VoyageFields(title: "Milestones", description: "Actual confirmed states"))
  state.voyages = [voyage]
  #expect(
    VoyageSyncProgress(voyage: voyage, workspace: state).milestones.map(\.complete) == [
      true, false, false, false,
    ])
  voyage.serverId = UUID()
  var photo = LocalPhoto(voyageId: voyage.id, filename: "Pending")
  photo.serverId = UUID()
  photo.transfer = .accepted
  state.photos = [photo]
  #expect(
    VoyageSyncProgress(voyage: voyage, workspace: state).milestones.map(\.complete) == [
      true, true, true, false,
    ])
  state.photos[0].variant = .ready
  state.publicationIntents = [
    try PublicationIntent.review(voyage: voyage, photos: state.photos, at: Date())
  ]
  #expect(
    VoyageSyncProgress(voyage: voyage, workspace: state).milestones.map(\.complete) == [
      true, true, true, true, false,
    ])
  voyage.confirmedPublished = true
  #expect(
    VoyageSyncProgress(voyage: voyage, workspace: state).milestones.allSatisfy { $0.complete })
  state.photos.append(LocalPhoto(voyageId: voyage.id, filename: "Not accepted"))
  let progress = VoyageSyncProgress(voyage: voyage, workspace: state)
  #expect(progress.acceptedCount == 1)
  #expect(progress.processedCount == 1)
  #expect(progress.photoCount == 2)
  state.photos[1].deleted = true
  #expect(VoyageSyncProgress(voyage: voyage, workspace: state).photoCount == 1)
}
