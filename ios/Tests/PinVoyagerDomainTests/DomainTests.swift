import Foundation
import Testing

@testable import PinVoyagerDomain

@Test func voyageValidationCountsUnicodeScalars() throws {
  #expect(throws: ValidationError.self) {
    try VoyageFields(title: " ", description: "x").validated(isNew: true)
  }
  #expect(throws: ValidationError.self) {
    try VoyageFields(title: "x", description: " ").validated(isNew: true)
  }
  #expect(throws: ValidationError.self) {
    try VoyageFields(title: String(repeating: "e\u{301}", count: 101), description: "x").validated(
      isNew: true)
  }
  #expect(throws: ValidationError.self) {
    try VoyageFields(title: "x", description: String(repeating: "x", count: 8_001)).validated(
      isNew: false)
  }
  let fields = try VoyageFields(title: "  North  ", description: " trip ").validated(isNew: true)
  #expect(fields.title == "North")
  #expect(fields.description == "trip")
  #expect(fields.granularity == .km1)
  #expect(try VoyageFields(title: "x", description: nil).validated(isNew: false).description == nil)
}

@Test func displayNamesMatchServerRules() throws {
  #expect(try DisplayName.normalize("  ALIce-9 ") == "alice-9")
  for value in ["ab", "1abc", "abc-", "ábc", "a_b", String(repeating: "a", count: 31)] {
    #expect(throws: ValidationError.self) { try DisplayName.normalize(value) }
  }
}

@Test func roundingMatchesRustAndFailsClosed() throws {
  let coordinate = Coordinate(latitude: 43.07, longitude: 141.35)
  #expect(Granularity.exact.apply(coordinate) == coordinate)
  let coarse = Granularity.km10.apply(coordinate)
  #expect(abs(coarse.latitude - 43.07402084081926) < 1e-10)
  #expect(abs(coarse.longitude - 141.36184241531976) < 1e-10)
  #expect(coarse.precisionM == 5_000)
  for precision in Granularity.allCases {
    for latitude in [-90.0, -85, -0.1, 0, 85, 90] {
      for longitude in [-180.0, -179.99, 0, 179.99, 180] {
        let result = precision.apply(Coordinate(latitude: latitude, longitude: longitude))
        #expect((-90...90).contains(result.latitude))
        #expect((-180...180).contains(result.longitude))
      }
    }
  }
  let unknown = try JSONDecoder().decode(Granularity.self, from: Data("\"future\"".utf8))
  #expect(unknown == .unsupported)
  #expect(unknown.apply(coordinate) == Granularity.km50.apply(coordinate))
  #expect(!unknown.canPublish)
}

@Test func linksAcceptOnlyTrustedStructuredDestinations() throws {
  let origin = try TrustedOrigin(URL(string: "https://pinvoyager.com")!)
  let id = UUID()
  #expect(
    AppRoute.parse(URL(string: "https://pinvoyager.com/voyages/\(id)")!, origin: origin)
      == .voyage(id))
  #expect(
    AppRoute.parse(URL(string: "https://pinvoyager.com/voyages/\(id)/edit")!, origin: origin)
      == .editVoyage(id))
  #expect(
    AppRoute.parse(URL(string: "https://pinvoyager.com/upload?voyage=\(id)")!, origin: origin)
      == .addPhotos(id))
  #expect(
    AppRoute.parse(URL(string: "https://pinvoyager.com/u/alice")!, origin: origin)
      == .profile("alice"))
  #expect(
    AppRoute.parse(URL(string: "https://pinvoyager.com/settings")!, origin: origin) == .account)
  #expect(
    AppRoute.parse(URL(string: "https://pinvoyager.com/upload")!, origin: origin) == .createVoyage)
  for url in [
    "https://evil.test/voyages/\(id)", "http://pinvoyager.com/upload",
    "https://user@pinvoyager.com/upload", "https://pinvoyager.com:444/upload",
    "https://pinvoyager.com/voyages/no", "https://pinvoyager.com/upload?voyage=no",
    "https://pinvoyager.com/u/../../auth",
    "https://pinvoyager.com/upload?voyage=\(id)&voyage=\(id)",
  ] {
    #expect(AppRoute.parse(URL(string: url)!, origin: origin) == nil)
  }
  #expect(throws: OriginError.self) { try TrustedOrigin(URL(string: "http://evil.test")!) }
  #expect(
    try TrustedOrigin(URL(string: "http://127.0.0.1:8080")!, allowsLocalHTTP: true).url.port == 8080
  )
  #expect(origin.url(path: ["voyages", id.uuidString]).host == "pinvoyager.com")
}

@Test func threeWayMergeRequiresExplicitConflictingChoice() {
  #expect(FieldComparison(base: "a", local: "a", remote: "b").resolution == .remote)
  #expect(FieldComparison(base: "a", local: "b", remote: "a").resolution == .local)
  #expect(FieldComparison(base: "a", local: "b", remote: "b").resolution == .local)
  #expect(FieldComparison(base: "a", local: "b", remote: "c").resolution == .needsChoice)
}

@Test func policyWaitsWithoutConsumingAttempts() {
  var policy = SyncPolicy()
  #expect(policy.waitReason(network: .offline, authenticated: true) == .offline)
  #expect(policy.waitReason(network: .cellular(constrained: false), authenticated: true) == .wifi)
  #expect(policy.waitReason(network: .wifi(constrained: true), authenticated: true) == .constrained)
  #expect(policy.waitReason(network: .wifi(constrained: false), authenticated: false) == .signIn)
  #expect(policy.waitReason(network: .wifi(constrained: false), authenticated: true) == nil)
  policy.wifiOnly = false
  policy.allowsConstrained = true
  #expect(policy.waitReason(network: .cellular(constrained: true), authenticated: true) == nil)
  policy.paused = true
  #expect(policy.waitReason(network: .wifi(constrained: false), authenticated: true) == .paused)
}

@Test func retryClassificationAndBackoff() {
  #expect(ResponseDisposition.classify(status: 401) == .signIn)
  #expect(ResponseDisposition.classify(status: 412) == .conflict)
  for status in [408, 429, 500, 503, 599] {
    #expect(ResponseDisposition.classify(status: status) == .reconcile)
  }
  for status in [400, 403, 409, 413, 415, 422] {
    #expect(ResponseDisposition.classify(status: status) == .attention)
  }
  #expect(ResponseDisposition.classify(status: 404) == .destinationMissing)
  #expect(ResponseDisposition.classify(status: 201) == .success)
  #expect(RetrySchedule.delay(attempt: 1, randomUnit: 1) == 2)
  #expect(RetrySchedule.delay(attempt: 30, randomUnit: 1) == 900)
  #expect(RetrySchedule.delay(attempt: 1, randomUnit: 0) == 0)
  #expect(RetrySchedule.delay(attempt: 1, randomUnit: 0.5, retryAfter: 60) == 60)
}

@Test func shortestLongitudeArcAndDiscoveryZoom() {
  let bounds = MapGeometry.bounds([
    Coordinate(latitude: 10, longitude: 179), Coordinate(latitude: 12, longitude: -179),
  ])!
  #expect(abs(bounds.longitudeSpan - 2) < 1e-8)
  #expect(abs(abs(bounds.longitude) - 180) < 1e-8)
  #expect(bounds.latitude == 11)
  #expect(MapGeometry.bounds([]) == nil)
  #expect(MapGeometry.discoveryZoom(width: 512, worldFraction: 1) == 0)
  #expect(MapGeometry.discoveryZoom(width: 512, worldFraction: 0.25) == 2)
  #expect(MapGeometry.discoveryZoom(width: 0, worldFraction: 0) == 0)
  #expect(MapGeometry.discoveryZoom(width: 512, worldFraction: 1e-12) == 20)
}

@Test func voyageClustersWrapAndKeepInputOrder() {
  let first = MapPhotoPin(id: UUID(), coordinate: Coordinate(latitude: 0, longitude: 179.99))
  let second = MapPhotoPin(id: UUID(), coordinate: Coordinate(latitude: 0, longitude: -179.99))
  let far = MapPhotoPin(id: UUID(), coordinate: Coordinate(latitude: 45, longitude: 0))
  let clusters = MapGeometry.cluster([first, second, far], zoom: 5)
  #expect(clusters.count == 2)
  #expect(clusters.first?.members == [first.id, second.id])
  #expect(MapGeometry.cluster([], zoom: 5).isEmpty)
  #expect(MapGeometry.cluster([first, second], zoom: 20).count == 2)
}

@Test func publicationFreezesReviewedSnapshotAndExpires() throws {
  let account = UUID()
  var voyage = LocalVoyage(
    ownerId: account, fields: VoyageFields(title: "Trip", description: "Winter"))
  var photo = LocalPhoto(voyageId: voyage.id, filename: "one.jpg")
  #expect(throws: ValidationError.self) {
    try PublicationIntent.review(voyage: voyage, photos: [], at: .distantPast)
  }
  #expect(throws: ValidationError.self) {
    try PublicationIntent.review(voyage: voyage, photos: [photo], at: .distantPast)
  }
  photo.source = SourceFile(
    relativePath: "one.jpg", contentHash: "abc", byteCount: 10, mediaType: "image/jpeg")
  photo.transfer = .saved
  let now = Date(timeIntervalSince1970: 10)
  let intent = try PublicationIntent.review(voyage: voyage, photos: [photo], at: now)
  #expect(intent.expiresAt.timeIntervalSince(now) == 7 * 24 * 60 * 60)
  #expect(intent.photoReferences == [.upload(key: photo.uploadKey, contentHash: "abc")])
  #expect(intent.status == .queued)
  #expect(!voyage.confirmedPublished)
  voyage.fields.granularity = .unsupported
  #expect(throws: ValidationError.self) {
    try PublicationIntent.review(voyage: voyage, photos: [photo], at: now)
  }
  voyage.fields.granularity = .km1
  photo.variant = .failed
  #expect(throws: ValidationError.self) {
    try PublicationIntent.review(voyage: voyage, photos: [photo], at: now)
  }
}

@Test func processingFailureCannotBecomeAnUploadRetry() {
  var photo = LocalPhoto(voyageId: UUID(), filename: "one.jpg")
  #expect(!photo.canRetryUpload)
  photo.transfer = .needsAttention
  #expect(photo.canRetryUpload)
  photo.variant = .failed
  #expect(!photo.canRetryUpload)
}

@Test func unknownStatesStayPrivateAndErrorsRemainActionable() throws {
  #expect(
    try JSONDecoder().decode(VariantState.self, from: Data("\"new-state\"".utf8)) == .unsupported)
  #expect(!Coordinate(latitude: .nan, longitude: 0).isValid)
  #expect(MapGeometry.bounds([Coordinate(latitude: 100, longitude: 0)]) == nil)
  #expect(MapGeometry.cluster([], zoom: .infinity).isEmpty)
  for error in [
    WorkspaceError.missing, .locked, .wrongAccount, .missingVoyage, .invalidOrder,
    .publicationArmed, .unsupportedProtocol, .malformedReceipt,
  ] { #expect(error.errorDescription?.isEmpty == false) }
  let validation = ValidationError(field: "title", message: "Enter a name")
  #expect(validation.errorDescription == "Enter a name")
  for precision in Granularity.allCases + [.unsupported] { #expect(!precision.label.isEmpty) }
  #expect(Coordinate(latitude: 0, longitude: 0).isValid)
  let profile = UserProfile(id: UUID(), displayName: "alice")
  var workspace = Workspace(account: LocalAccount(profile: profile))
  var photo = LocalPhoto(voyageId: UUID(), filename: "Missing")
  photo.transfer = .needsSource
  workspace.photos.append(photo)
  var operation = OutboxOperation(
    accountId: profile.id, generation: 0, voyageId: photo.voyageId, kind: .deleteVoyage)
  operation.state = .conflict
  workspace.operations.append(operation)
  #expect(workspace.attentionCount == 2)
  #expect(workspace.transfers.isEmpty)
  #expect(
    VoyageWorkingCopy(
      voyage: LocalVoyage(ownerId: profile.id, fields: VoyageFields()), fields: VoyageFields()
    ).id != profile.id)
}

@Test func attentionBadgesIncludeRetainedSourcesWithoutCountingTheirUploadTwice() {
  let account = UUID()
  let voyage = UUID()
  var workspace = Workspace(account: LocalAccount(profile: UserProfile(id: account)))
  var photo = LocalPhoto(voyageId: voyage, filename: "Retained original")
  photo.transfer = .needsAttention
  workspace.photos = [photo]
  #expect(workspace.attentionCount == 1)
  var upload = OutboxOperation(
    accountId: account, generation: 0, voyageId: voyage,
    kind: .upload(photoId: photo.id, publicConsent: false))
  upload.state = .attention
  workspace.operations = [upload]
  #expect(workspace.attentionCount == 1)
  workspace.photos[0].deleted = true
  #expect(workspace.attentionCount == 1)
  workspace.operations[0].state = .applied
  #expect(workspace.attentionCount == 0)
}
