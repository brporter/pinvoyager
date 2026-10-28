import Foundation

public struct VoyageSyncProgress: Sendable {
  public struct Milestone: Identifiable, Sendable {
    public var id: String { title }
    public let title: String
    public let complete: Bool
  }
  public let photoCount: Int
  public let acceptedCount: Int
  public let processedCount: Int
  public let milestones: [Milestone]
  public init(voyage: LocalVoyage, workspace: Workspace) {
    let photos = workspace.photos(for: voyage.id)
    photoCount = photos.count
    acceptedCount = photos.filter { $0.serverId != nil }.count
    processedCount = photos.filter { $0.serverId != nil && $0.variant == .ready }.count
    var steps = [
      Milestone(title: "Saved on device", complete: true),
      Milestone(title: "Draft on website", complete: voyage.serverId != nil),
      Milestone(title: "Photos accepted", complete: photoCount > 0 && acceptedCount == photoCount),
      Milestone(
        title: "Photos processed", complete: photoCount > 0 && processedCount == photoCount),
    ]
    if voyage.confirmedPublished
      || workspace.publicationIntents.contains(where: { $0.voyageId == voyage.id })
    {
      steps.append(Milestone(title: "Published", complete: voyage.confirmedPublished))
    }
    milestones = steps
  }
}
