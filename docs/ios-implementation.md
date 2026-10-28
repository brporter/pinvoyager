# Native iOS implementation status

The native implementation follows [the iOS design](ios-app-design.md), with the
defaults requested by the implementation task: iOS 18, SwiftUI, MapKit, Core
Data, provider authentication, and local saving before synchronization.
The project is available for local development and simulator testing;
physical-device, provider, and deployment acceptance remains unfinished.
The API implements native authentication,
revision-checked mutations, durable receipts, upload cancellation, frozen publication
intents, and separate upload deadlines. See [native API behavior](native-api.md).

## Design review

The implementation separates durable local saving, transfer acceptance, variant
processing, and publication confirmation. The app checks `/api/client-capabilities`;
provider handoff is advertised independently of synchronization. The implemented API
now advertises synchronization protocol version 1. Against
an older server, a capabilities 404 permits public browsing and reports unavailable
native authentication/synchronization. Existing-account local editing remains durable.

The API keeps browser cookies and native bearer sessions separate. Native authentication
reuses provider verification, browser binding, account policy, and identity linking.
Mutation receipts survive resource deletion. Every voyage writer increments the
content revision; variant processing does not. Publication waits for exactly the
reviewed membership and order and stops after unrelated edits. A historical publication
receipt triggers a current visibility check before the app reports a voyage as public.

## Layers and current behavior

| Layer | Implementation and checks |
| --- | --- |
| Domain | `ios/Sources/PinVoyagerDomain/`: value types, scalar-based field/name validation, public rounding, wrapped map geometry and voyage clustering, trusted links, revision-checked request values, transfer/publication states, retry policy, three-way field comparison. `DomainTests` checks meaningful privacy and validation outcomes. |
| Persistence | `CoreDataWorkspaceStore` implements `WorkspaceStore` using a private context and atomic outbox transactions. `LocalRepository` implements local commands, scratch drafts and working copies, complete ordering, removals/tombstones, publication review/cancellation, blocking/reporting queues, and explicit metadata-conflict resolution. Tests reopen actual SQLite stores, preserve corrupt stores, check rollback and account isolation, and exercise concurrent saves. |
| Files/media | `FileVault`, `PhotoPreparer`, and `MultipartBody` stream originals, hash files, check format/size/reserve space, protect files, create metadata-free downsampled previews, retain EXIF in source bytes, and build file-backed multipart requests. Synthetic HEIC and generated GPS/date fixtures check the resulting files. |
| Transport/security | `APIClient`, `HTTPTransport`, `CredentialStore`, and `NativeAuthentication` compose separately. Foreground sessions do not share browser cookies or ordinary image caches, refuse redirects, and validate origins before attaching credentials. PKCE/state and single-use pending attempts protect app callbacks. Keychain errors propagate. Tests use HTTP stubs, injected Security calls, and a signed simulator Keychain. |
| Synchronization | `SyncCoordinator`, `SyncRemote`, `RequestFactory`, `BackgroundUploadService`, and `TransferDriver` implement bounded scheduling, immutable submitted requests, durable dependencies, receipt reconciliation, retry limits/backoff, generation checks, publication-status polling, and suspended task creation followed by durable task IDs before resume. Deterministic tests cover lost responses, restrictions, stale revisions, consent expiry, and existing-task adoption. |
| Presentation | `ios/PinVoyager/`: four-tab SwiftUI shell, iPad sidebar, discovery list/MapKit clusters, public detail/profile/gallery, My Voyages, durable voyage editor, Photos picker, AVFoundation still capture, private/public local-map preview, accessible photo reordering, publication review, sync status, provider/name/account screens, settings/export/help, queued reports and blocks, and metadata-conflict comparison. `AppModelTests` checks commands and account/reconciliation behavior; `NavigationTests` exercises native controls. |

The application imports no web UI. Semantic assets use the shared teal and orange
colors and Horizon Pin logo. System navigation, forms, Photos picker, authentication
presentation, MapKit attribution, camera APIs, and sharing supply native controls.
URLs shared by a confirmed online public detail view use trusted build
configuration. Local previews apply `Granularity.apply` before clustering; exact
coordinates appear only in private owner surfaces. Public image requests never
fetch originals or treat an HTTP cache entry as an offline authorization grant.

Outbox failures stop affected voyages on conflicts or permanent restrictions.
Transient failures become uncertain, require receipt reconciliation, use full
jitter backoff and `Retry-After`, and stop after eight attempts. No network path
hint proves server availability. A 401 pauses remote work and preserves local
files. Signing out locks the workspace, increments its generation, cancels tasks,
and stores a revocation-only credential obligation before reconnecting. Another
account cannot inherit those drafts.

Metadata-conflict resolution requires the current owner-visible server revision
and explicit choices for conflicting fields. It retains the immutable original
request and creates a successor against the reviewed revision. Recovery into a
new private voyage assigns fresh voyage/photo/upload identities and includes only
retained source files; it does not invent missing originals.

Signed-out network preferences persist through `DevicePreferences` without creating an
account workspace. New account workspaces inherit these defaults; existing accounts
keep their own policy. Publication notifications still require account-specific opt-in.
Corrupt preference data remains available for diagnosis and produces conservative
network defaults and a visible error. `DevicePreferencesTests` and
`DevicePreferencesModelTests` cover persistence, account separation, and failures.

Each import receives a durable batch identity. An explicit cellular exception applies
only to that batch's pending photo identities, expires after receipt acceptance or
removal, and never overrides Low Data Mode or pause settings. Settings lists pending
batches with remaining byte estimates and allows revocation. Policy changes increment
the account generation and cancel saved sessions, including while offline. Background
sessions and response evidence distinguish ordinary and cellular-permitted tasks;
`BatchUploadTests`, `BackgroundTransferTests`, `TransferDriverTests`, and
`PhotoImportPresentationTests` cover the boundaries and restart behavior.

Removing a photo cancels its own system upload task even when offline. Synchronization
still reconciles its receipt, but never submits its bytes again. Confirmed removal
settles the original upload while preserving its immutable request and retained source.
`removingAnUncertainUploadCancelsItsTaskWithoutReplayingBytesOrRequiringSignIn` and
`testLocalRemovalCancelsSavedUploadTasksEvenWhenNoCoordinatorCanRunOffline` check these
paths. Cancellation does not remove another photo's task.

`SceneRouter` belongs to each `RootView`, while `AppModel` and synchronization remain
shared. Each window restores its tab and typed navigation paths through SceneStorage,
including local voyage IDs and pending authentication routes. Restoring another
account clears private destinations. Explicit authentication cancellation clears the
intended route. Deep links first await the editor's durable working-copy save; failed
saves retain the link for retry. `SceneRouterTests`, the initial-name route test in
`AppModelTests`, and native navigation journeys check this behavior.

`TemporaryImageCache` stores ordinary variants under hashed account/origin keys with
a 250 MiB default limit and least-recently-used eviction. `APIClient.image` performs
an online request for every later read and uses cached bytes only after an authorized
304. A failed or denied request removes that cache entry and never displays cached
bytes as a fallback. Explicit owner downloads and source files use separate storage.
`ImageCacheTests` checks eviction, restart, conditional requests, and account isolation.

`StorageManager` reports actual source/download sizes and only removes originals after
all references to that source are accepted, ready, free of pending voyage operations,
and backed by a local display copy whose bytes match its recorded hash. Cleanup keeps
shared sources needed by pending work. It clears metadata transactionally before
removing eligible files. A crash after that transaction may leave a recoverable orphan
file; it cannot remove a pending source. `StorageTests` checks these conditions and
missing/corrupt owner files. Storage settings require cleanup confirmation and retain
originals by default. Opting out also checks eligibility after synchronization and owner
downloads. `ExportVoyageView` previews a selected voyage's files and missing originals,
then creates a ZIP using Foundation’s coordinated upload snapshot and offers native sharing and the Files document picker. The picker receives a file URL rather than an in-memory archive. Exports preserve exact private
metadata and include removed photos' retained sources.

`ConflictReviewService` persists field choices, photo-order choices, the reviewed
website revision, and visibility choices in the protected workspace. An offline Save
stores that working copy without preparing a server request. Online Save reloads an
owner snapshot; a newer website revision updates the comparison and preserves choices
for another explicit review. A conflicted unpublication offers Keep website visibility
or a new revision-checked Make private request. It never changes confirmed visibility
before server acknowledgement. `ConflictReviewTests`, `OwnerSnapshotTests`, and
`SyncFlowTests` cover these outcomes. Normal editor working copies remain separate.

`SharedRequests` deduplicates concurrent image and profile reads without retaining a
response as an authorization grant. Known display names and aliases map to the stable
profile account ID, while viewer accounts, credentials and pagination remain separate.
`profileNamesShareConcurrentRequestsAndKnownAliasesUseTheSameAccountIdentity` checks
normalization, aliases, concurrent readers and authenticated separation.

`PhotoGallery` supplies common native owner/public gallery controls and lazy paging and
thumbnail views. `GalleryNavigation` keeps the selected photo ID when a refresh reorders
photos and selects the nearest surviving position after removal. `ZoomablePhoto` reports
zoom state so the gallery pauses page scrolling while the image is zoomed. Owner
information labels exact locations as private; public information uses public rounded
data. Located photos offer Show on map; other travelers' photos offer reporting.
`LocalVoyageView` and `RemoteVoyageView` present galleries with `fullScreenCover`
from both photo lists/grids and map markers. The gallery fills the current app
window on iPhone and iPad; it no longer opens in a small sheet or at a compact
map detent. Close dismisses the cover, and map galleries keep their selected
photo membership. VoiceOver focus returns to the invoking thumbnail on dismissal.
`GalleryNavigationTests`, `PhotoZoomTests`,
`PublicBrowsingTests` and `LocalVoyageFlowTests` check the main paths.

The gallery-size regression uses `XCTestCase.assertGalleryFillsWindow` in
`ios/PinVoyagerUITests/AccessibilityActions.swift` to compare the gallery's
navigation width and top/bottom controls with its window. On October 27, the
initial iPad run measured a 580-point gallery in a 1,032-point window and failed
for owner/public galleries opened from photos and maps. The tests retain before
and after screenshots. The fix replaces the two `.sheet` presenters and removes
their detent and drag-indicator modifiers; `PhotoGallery` continues to use native
navigation and a viewport-relative paged layout.

The test-first run is saved in `20261027-gallery-before.xcresult`: all three
extended UI cases failed the new size assertions before the presentation change.
After the change, `20261027-gallery-after-ipad.xcresult` passes all three cases,
covering owner/public galleries opened from both photos and map markers. The
retained screenshots show the map gallery changing from a narrow bottom sheet
to a cover that fills the app window.

`PrecisionPickerView` keeps its selection separate until Done; Cancel preserves the
editor's previous choice. `PrecisionPreview` applies the same rounding algorithm to a
saved photo or fixed sample without requesting location access. It draws the cell
approximation's diagonal radius, labels exact choices, and shows an explicitly schematic
offline example. Public photo grids use one column at accessibility text sizes.
`testPrecisionReviewUsesAnOfflineExampleAndCancelKeepsThePreviousChoice` checks the
offline review and confirmation flow.

`AuthenticationPresentation` composes the native exchange actor with a
`BrowserAuthenticating` interface and shares sign-in, linking and deletion
reauthentication handling. It prevents concurrent button presses, clears canceled
attempts, keeps failures visible and stops its progress state after every outcome.
`BrowserAuthentication` uses the invoking window captured by `WindowContext`, and
settles its continuation once even if session startup and completion overlap.
`AuthenticationPresentationTests` covers completed exchanges, linking, browser
cancellation/start failure, and purpose-bound deletion proofs with synthetic sessions.
These tests do not establish real-provider acceptance.

Network/storage toggles update one `SyncPreference` in a store transaction. They read
the current policy instead of retaining a screen-local snapshot, and ignore actions
from an old account window. `concurrentAccountPreferencesNeverRestoreAnOlderToggleSnapshot`
and `DevicePreferencesModelTests` check concurrent changes, generation increments,
account separation, and preservation of other choices. The UI test for changing Wi-Fi
after the originals preference also passes; it did not reproduce the concurrent-write
case before the change.

`ProfilePresentation` retains existing cards and the pagination cursor after a transient
load-more failure, and Retry loading more requests the same cursor. Authorization
failures and offline/access changes clear the public view. The API client rejects a
canonical profile response whose account ID does not match the requested ID. Profile
screens also offer traveler reporting and blocking. `ProfilePresentationTests` and
`canonicalProfilesRejectAnotherTravelersIdentity` cover these boundaries.

The system Photos-picker journey now runs against seeded simulator media. Explicitly
booting the simulator before `simctl addmedia` resolved the earlier seeding failure.
`PhotoPickerTests` verifies the real transferable file copy, durable import and retained
batch cellular permission. `NotificationFlowTests` grants the system opt-in and checks
a confirmed publication's foreground banner. The notification adapter uses a Sendable
notice value and actor-owned system center; it checks pending/delivered identifiers,
rechecks authorization, and presents foreground banners through the app delegate.
These simulator checks supplement the physical-device acceptance matrix.

`AppEnvironment` composes paths, transports, credentials, preferences, account recovery
and system transfers. `BootstrapTests` now exercises the production startup sequence
with isolated injected services and checks that invalid configured origins produce a
recoverable error. `ExportFilesPicker` uses the native document picker to export the
reviewed ZIP. Opening and canceling that dialog passes; the earlier transferable-based
export exposed an empty suggested-filename exception in the installed SwiftUI runtime.
The ZIP unit checks verify a readable snapshot after file coordination ends and retain
an existing archive and original sources when copying fails.

`VoyageEditSession` binds working-copy saves, discard, and commit to the account
that opened the editor. Armed publications permit a separate local working copy
without altering reviewed fields or publication consent. Completing an import
resets the import screen's cellular toggle; saved batch permission remains scoped
to that batch. `VoyageEditingTests`, `EditorFlowTests`, and `PhotoPickerTests`
cover those boundaries. The session API tests first failed for missing behavior;
the armed-edit and cellular-reset UI regressions failed before their fixes.

`PhotoArrangement` stages complete photo orders and selection using stable IDs.
`ManagePhotosView` offers native drag handles, explicit earlier/later controls,
Save/Cancel order, bulk selection/removal, and import. Arrivals append without
replacing staged moves. `LocalRepository.removePhotos` validates the whole
selection in one transaction and retains source and uncertain-upload evidence.
`PhotoArrangementTests`, `VoyageEditingTests.bulkPhotoRemovalIsAtomicAndKeepsUncertainUploadEvidenceAndSources`,
and `PhotoManagementTests` cover order cancellation, saving, and bulk removal.
The initial domain and UI runs failed for the absent arrangement and controls;
the matching focused runs pass.

`LocalVoyage.uploadsPaused` is an optional backward-compatible payload field.
`Workspace.uploadPolicy(for:)` combines account policy, batch consent, and voyage
pause. Metadata and receipts remain eligible while a voyage's bytes are paused.
`BackgroundUploadService` checks pause again before persisting/resuming a system
task; `BackgroundTransferRecovery` rejects paused adoption and cancels only
ineligible operations. `VoyageSyncTests` and the transfer race in
`BackgroundTransferTests.voyagePauseBetweenTaskCreationAndResumeCancelsOnlyThatTask`
failed before implementation and pass afterward.

`DraftRecoveryService.recover` hashes independent local copies before creating a
new private voyage with new photo/upload identities. Missing or corrupt material
never enters its upload queue. It falls back to verified full/display owner
downloads, labels those as processed copies, and does not invent embedded capture
metadata. The previous voyage and its evidence remain intact. `DraftRecoveryTests`
checks originals, corrupted originals, processed fallback, missing files, and locked
accounts; its missing-service run failed before implementation and passes afterward.

`LocalDiscardService` distinguishes definite local-only deletion from explicit
discard of an unavailable conflicted destination. It tombstones first, cancels
transfers, clears local file references, and removes only files with no remaining
reference in that account. It preserves immutable request evidence. An optional
`Workspace.pendingFileRemovals` journal persists interrupted removals in Core Data
preferences; startup retries that journal after background recovery.
`LocalDiscardTests` covers shared files, cancellation ordering, ambiguous creation,
locked accounts, and restart cleanup, with missing-API red runs and passing green
runs. File removal failures leave the journal available for retry.

`SyncActivity` groups voyage operations under Needs attention, In progress,
Waiting, and Completed. `SyncCoordinator.apply` records the first local receipt
completion date; repeat reconciliation does not extend it. The activity view
omits completed entries after seven days and leaves their immutable requests,
receipt edges, and operation identities in the correctness ledger. Older applied
records without a known completion date are retained but are not shown as recent.
`SyncActivityTests` and `SyncTests.receiptCompletionDatePersistsAndReplayDoesNotExtendActivityRetention`
cover grouping, retention boundaries, and repeated reconciliation; the receipt-date
regression failed before implementation and passes afterward. `VoyageSyncProgress`
reports only confirmed saved, remote, accepted, processed, and publication milestones;
`VoyageSyncProgressTests` covers empty, pending, ready, deleted, and confirmed-public
states. The UI provides per-voyage pause independently of the account-wide pause.

Within each activity group, `SyncActivity.rows` sorts operations by immutable ID.
Core Data fetch order does not define presentation order. The regression in
`ios/Tests/PinVoyagerDomainTests/SyncActivityTests.swift`,
`syncActivityOrderDoesNotDependOnTheStoresFetchOrder`, first reproduced different
row orders for equivalent store snapshots and passes after sorting at the
presentation boundary.

`PublicStateTests` covers empty voyages and own profiles, processing versus failed
photos, unavailable voyage retry, and invalid image bytes. The invalid-image test
first found a gallery that remained on its loading indicator after a successful
HTTP response containing undecodable bytes. `GalleryImage` now displays a decoding
error. `StorageFlowTests` exercises actual protected owner-download saving/removal
and lists unavailable originals before ZIP export. `SceneRouterTests` verifies
owner-only edit/import links and rejects foreign/unmapped destinations.

`SyncTests.preparingARequestCannotOverwriteAnEditOrDispatchAnOperationCanceledBeforeItsCheckpoint`
pauses the store before a prepared request becomes durable, then edits or cancels
the operation. Its red run reproduced stale metadata submission and dispatch of
canceled work. `SyncCoordinator.run` now compares the current operation's kind,
state, and request before storing the checkpoint, then verifies that checkpoint
and the account generation before dispatch. The matching green run passes and
keeps newer fields waiting without treating an ordinary local edit as an error.

`AppModel.removeEligibleOriginals` applies the explicit original-storage opt-out
after both ordinary owner downloads and full-size downloads, and after sync.
`AppModelTests.testFullOwnerDownloadsReleaseOnlyEligibleOriginalsAfterExplicitStorageOptOut`
first failed because full-size downloads skipped cleanup, then passed after
sharing that policy helper. It verifies that a downloaded verified owner copy
permits removal while another photo's retained source remains protected.
`PresentationAdapterTests` covers invoking-window anchors, Files destination and
cancellation callbacks, and double-tap zoom reset. `ProfileFlowTests` covers
pagination deduplication, report queuing, immediate local blocking, and guest
sign-in before blocking. These tests use synthetic accounts and local files.

`LocalRepository.undoPhotoRemovals` restores a selected batch in one transaction;
if any removal has a submitted request, the whole undo fails. `VoyageDeletionUndo`
stores the photo IDs and pending operation states that existed before a voyage
was deleted. `undoVoyageDeletion` restores that snapshot only while its deletion
request remains unsent. It preserves earlier photo removals and completed ledger
entries. `Workspace.canUndoPhotoRemoval` and `canUndoVoyageDeletion` drive the
controls in `ManagePhotosView`, `LocalVoyageView`, and `MyVoyagesView`. Queued
voyage deletions remain visible for Undo after reopening because their state is
stored with the voyage. `VoyageEditingTests` began red for the missing APIs and
then caught a completed operation being changed to canceled; those cases pass
after implementation. The corresponding UI journeys cover bulk and voyage Undo.

`AppModel.cancelPublicationAndSignOut` keeps the session usable until cancellation
is confirmed. If publication wins the race, `AccountView` presents explicit
Unpublish and sign out, leave public and sign out, and keep account open choices.
`unpublishAndSignOut` requires confirmed private visibility before locking the
workspace and revoking the session. An armed or unanswered publication requires
an explicit leave-scheduled choice to sign out without that confirmation.
`AppModelTests` covers canceled, still-processing, already-published, offline,
and rejected-unpublication outcomes. `AccountFlowTests` exercises the real native
HTTP adapter with synthetic receipts when publication wins cancellation.

`Workspace.publicationMayProceed` includes an unanswered submitted publication,
including one awaiting user attention. Such work protects the reviewed fields and
photo list while allowing a separate editor working copy. An optional
`OutboxOperation.receiptRejected` records an authoritative negative receipt;
only that proof permits local cancellation or superseding the old request.
Explicit cancellation resumes receipt checking for an unanswered request that
needs attention. When its receipt proves rejection, the coordinator settles an
unsent cancellation without inventing a server intent ID. Tests in
`VoyageEditingTests` first reproduced mutation of an unanswered snapshot and a
stale cancellation-pending flag; the repaired cases preserve immutable request
bytes and permit a fresh review after a proven rejection. The editor UI journey
checks the uncertainty message, working-copy save, disabled photo changes, and
explicit leave-scheduled sign-out choice.

`SyncPolicy.uploadsPaused` is an optional backward-compatible preference. Sync's
Pause uploads control cancels byte transfers and advances the account generation,
while receipt checks, metadata, and privacy changes remain eligible. The separate
Pause synchronization setting still stops all remote work. The per-voyage pause
combines with this upload policy. `BatchUploadTests.globalUploadPauseStopsBytesWithoutPausingMetadataOrReceiptRecovery`
checks receipt reconciliation and a metadata write while bytes are paused;
`VoyageSyncTests` checks pause/resume and independent settings values.

`LegacyWorkspaceTests.aVersionOnePendingRequestReopensWithoutNewOptionalFieldsAndReconcilesBeforeReplay`
uses a fixed version-one payload containing an uncertain create request, without
the new optional fields. It reopens a SQLite store and verifies that operation ID,
request bytes, and receipt-first recovery survive. The optional JSON additions do
not change Core Data's physical entity schema. `SceneRouter.requestCreation`
retains a guest Create action through sign-in and display-name setup; its route
regression began red for the missing shared action and passes after both Create
buttons use it.

`SyncCoordinator.run` reconciles every eligible submitted request before preparing
new revision-checked writes. Completed ledger records and the 100-write scheduling
limit do not limit receipt recovery. The regression
`BatchUploadTests.receiptRecoveryIncludesEverySubmittedPhotoAfterALargeCompletedHistory`
first reproduced 150 uncertain uploads being skipped behind 200 completed records.
It passes after removing the receipt-loop prefix and verifies that upload pause
and Wi-Fi-only restrictions do not prevent reconciliation or resend photo bytes.
New write scheduling remains bounded to 100 operations per pass.

The UI test helper `XCUIElement.reveal` checks the target's frame as well as
hittability. SwiftUI lists can expose prefetched controls before they are visible.
The helper chooses direction from the target position and adjusts drag distance
to avoid overshooting. The retry journey uses fixed operation identities, requires
the control to be clear of navigation/tab bars, and retains its pre-tap screenshot.
The earlier full run failed that journey; the repaired focused test passes three
consecutive iterations. The production retry command was unchanged.

`VoyageEditor` offers both Add photos and Save draft and close for a new voyage.
Both use the same validated local commit. Add photos records the new local ID in
`SceneRouter.creationCompleted`; `RootView` calls `creationDismissed` after the
editor closes, then opens that voyage's import screen. It does not require a
server ID or connection. Switching accounts clears the pending action.
`SceneRouterTests.testCreationOpensImportOnlyAfterTheEditorDismissesAndCannotCrossAccounts`
checks presentation ordering, one-time consumption, ordinary close, and account
separation. The offline creation UI regression first failed for the missing action.
Its focused green run passes with all eight router tests and both map journeys.

The native description editor has a bounded 180-point height and scrolls long
text internally. The regression in `ios/PinVoyagerUITests/EditorFlowTests.swift`,
`testInvalidRecoveredDraftShowsLimitsAndKeepsItsFieldsAfterFailedSaving`, first
measured a 4,790-point editor for a recovered description. It checks that the
editor stays below half the window height, scalar counters remain reachable,
failed validation retains both fields, and explicit discard removes the scratch.
The bounded-editor green run passes all five editor journeys, offline creation,
and the deterministic retry journey.

`DiscoverView` retains its Recent list and, after first use, its Map view when
switching modes. The inactive view ignores input and is hidden from accessibility;
inactive map tasks stop instead of requesting another viewport. Keeping the native
views alive preserves list scrolling and map camera state.
`DiscoveryMapTests.testSwitchingBetweenRecentAndMapPreservesTheRecentScrollPosition`
first reproduced the list resetting to its first card after a mode switch.
It passes after preserving the native views while hiding inactive content.

## Release acceptance still required

| Area | Required check or operator action |
| --- | --- |
| Providers and links | Configure real provider callbacks and signing; exercise sign-in, linking, deletion reauthentication, and Apple revocation against a test deployment. Deploy AASA and verify universal links on a physical device. |
| Background recovery | Without a debugger, interrupt real system uploads through suspension, termination, force-quit, reboot, and Wi-Fi/cellular changes. Verify receipt recovery, generation changes, cancellation, and deletion before another byte submission. |
| Files and permissions | Exercise iCloud-only Photos selections, camera/location denial, HEIC/WebP display, low disk capacity, interrupted import, recovery/export, and owner downloads on physical devices. Confirm exact metadata stays in owner surfaces and exported originals. |
| Accessibility and performance | Run VoiceOver, maximum Dynamic Type, Reduce Motion, iPad navigation, a 1,000-photo draft, and a 100-photo import batch. Measure responsiveness and memory on supported hardware. |
| Compatibility and deployment | Test the oldest supported iOS runtime. Rehearse additive API migration and rollback with pending device operations. Version-one SQLite fixtures verify optional payload additions; no prior production iOS schema exists. |
| Distribution and operations | Reconcile App Store privacy declarations with the deployed backend; complete operator legal/support details and moderation procedures before public distribution. No App Store release or server deployment was performed. |
| Hosted verification | Run the native CI job on a pushed branch. It enforces the full Swift threshold, checks bundled legal sources and formatting, and builds Release for Simulator; local results do not establish a successful hosted run. |

Version one uses named Core Data entities with Codable payloads rather than
individual searchable attributes for every domain field. A whole workspace
transaction preserves cross-entity consistency through the serialized writer.
The schema is explicitly versioned `PinVoyager.1`; future payload/schema changes
must include migration fixtures containing pending operations. There is no prior
production iOS schema and no claim that a future migration has already been tested.

## Verification record

The implementation began with domain tests failing for missing types, then store,
network/sync, background-task, media, scratch, recovery, publication-polling,
conflict-resolution, and credential-failure tests failing for their missing
behavior. The matching package suites passed after implementation. Simulator
journeys additionally found and corrected missing sheet environment propagation,
an unsigned Keychain test host, and draft-discard ordering. Test commands and
coverage/report handling are in [iOS running](ios-running.md).

On October 26, 2026, Xcode 27.1 and the iOS 27.0 simulators verified **255 native
unit tests and all 55 phone UI journeys**, plus iPad guest navigation. The full
phone bundle `20261026-181123.xcresult` passed all unit tests and 53 UI journeys.
Two UI tests accessed a Sync-details link or photo-count header outside the
current scroll position. After adding explicit scrolling, both pass in
`20261026-181123-ui-rerun.xcresult`; production code was unchanged between these
runs. `20261026-181123-ipad.xcresult` passes the iPad check. The fast macOS package
suites also pass: 18 domain tests and 160 data tests.

The matching profiles measure **17,422 / 18,253 production Swift executable lines
(95.45%)**, exceeding the 95% requirement with no production exclusions or missing
files. The coverage script includes the application, system adapters, SwiftUI
views, and embedded domain/data frameworks. The final measurement command is:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  python3 ios/Scripts/coverage.py ios/TestResults/20261026-181123.xcresult \
  --derived-data ios/.derivedData \
  --profile ios/TestResults/20261026-181123.profdata \
  --profile ios/TestResults/20261026-181123-ui-rerun.profdata \
  --profile ios/TestResults/20261026-181123-ipad.profdata
```

The generated `20261026-181123-coverage-swift.json` records aggregate and per-file
counts. The normal complete-run command remains `ios/Scripts/test.sh`; this
verification used its full phone run, the two corrected test reruns, and the
separate iPad check described above.

On October 27, the full phone run `20261027-205125.xcresult` passed all **255 unit
tests and 54 of 55 UI journeys**, including the gallery size assertions. The
existing `EditorFlowTests.testArmedPublicationAllowsASeparateWorkingCopyWithoutChangingTheReviewedVoyage`
failed its exact-title assertion with an extra trailing space, both in the full
run and in `20261027-205125-editor-rerun.xcresult`. Its scroll gestures ran while
the keyboard was open. Sending Return after typing dismisses the keyboard before
scrolling to Save; the exact assertion remains unchanged and passes in
`20261027-205125-editor-keyboard.xcresult`. Production sources were unchanged
between those runs. `20261027-205125-ipad.xcresult` passes guest navigation;
the three iPad gallery journeys pass in `20261027-gallery-after-ipad.xcresult`.

The matching October 27 profiles measure **17,426 / 18,241 production Swift
executable lines (95.53%)**, with all 70 production Swift files included and no
exclusions. `20261027-205125-coverage-swift.json` records the counts. The Simulator
Release build, strict Swift formatting, and `git diff --check` also pass. Backend
and website sources were unchanged for the gallery correction; their coverage
checks below were not rerun for this change. The current native measurement is:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  python3 ios/Scripts/coverage.py ios/TestResults/20261027-205125.xcresult \
  --derived-data ios/.derivedData \
  --profile ios/TestResults/20261027-205125.profdata \
  --profile ios/TestResults/20261027-205125-editor-keyboard.profdata \
  --profile ios/TestResults/20261027-205125-ipad.profdata \
  --profile ios/TestResults/20261027-gallery-after-ipad.profdata
```

The complete stable Rust check, `cargo coverage -- --test-threads=1`, passes: **98.56% lines,
98.90% functions, and 95.02% regions**. The expanded checks cover receipt/resource
atomicity, publication/cancellation failures, block/moderation failures, deletion
proof and removal rollback, malformed native headers, provider metadata and revocation
failures, unavailable photo storage, and conditional reads of missing variants.
The readiness/signal-registration regression failed before the fix and passes after
starting signal observation before binding the listener. No coverage exclusions were
added. Website checks, including `npm --prefix web run test -- --coverage`, pass
with **574 tests**, **99.10% statements, 98.88% branches,
98.32% functions, and 99.34% lines**. Node 24 test workers disable experimental
file-backed web storage so jsdom supplies browser storage. Web types, lint, and
production build pass; the existing bundle-size warning remains.

Tests ran with PostgreSQL on local port 5433 and the repository's native image tools.
Debug and Release simulator builds and strict Swift formatting pass. Rust formatting,
Clippy with warnings denied, documentation with warnings denied, and Release-mode
authentication/administration tests pass. Bundled legal-source verification and the
association-generator tests pass. No physical-device, provider-backed, deployed AASA,
or real background relaunch acceptance is claimed. Ignored `.xcresult` and coverage
files retain the local results.

