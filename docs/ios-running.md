# Running and testing the iOS client

The native client is in `ios/PinVoyager.xcodeproj`, with the shared `PinVoyager`
scheme and `PinVoyager.xctestplan`. It targets iOS 18 or later, iPhone and iPad,
and Swift 6. It uses system frameworks only. Xcode resolves the local
`ios/Package.swift`; there are no third-party Swift dependencies.

Public browsing uses the existing API. Native sign-in and remote mutations use
the implemented native API protocol; apply the migrations and configure provider
callbacks before connecting a real account.
Read [implementation status](ios-implementation.md) before using a real account
or treating this build as a release candidate.

## Xcode and configuration

Open `ios/PinVoyager.xcodeproj` and select the PinVoyager scheme and an iPhone or
iPad simulator. The implementation was built with Xcode 27.1 and tested on the
iOS 27.0 iPhone 18 Pro simulator. Older supported OS versions require separate
acceptance checks; successful compilation against the deployment target does
not establish their runtime behavior.

When the system selects command-line tools rather than full Xcode:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
xcodebuild -version
xcrun simctl list devices available
```

Set `PINVOYAGER_API_ORIGIN` and `PINVOYAGER_WEB_ORIGIN` in Xcode build settings.
Defaults use `https://pinvoyager.com`. They are independent trusted origins.
The API may return relative image paths but cannot redirect credentials to
another origin. Debug builds allow HTTP only for localhost or loopback; release
builds require HTTPS. For the simulator's local API, set the API origin to
`http://127.0.0.1:8080`. Physical devices need a reachable HTTPS development
endpoint; loopback on a device refers to that device.

Select your signing team for physical-device installation. Configure a separate
development bundle identifier, the `pinvoyager` authentication callback scheme,
the provider/server callback allowlist, and associated domains together. The
production host still needs an `apple-app-site-association` deployment. The
entitlement alone does not establish working universal links.

## Tests and coverage

Fast domain/data checks run on macOS without a simulator:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  swift test --package-path ios --scratch-path ios/.build-xcode --enable-code-coverage
```

The Xcode unit-test target also compiles these same test sources. It bundles the
existing Rust response fixtures and HEIC fixtures directly from their source
directories rather than maintaining native copies. The native test target adds
main-actor application-model tests, and the UI target checks navigation and
local authoring. The real Keychain test runs on iOS; macOS tests inject the
Security interface to check credential errors without relying on a logged-in
desktop Keychain.

Run the complete suite and the 95% executable-line coverage check:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
ios/Scripts/test.sh YOUR_IPHONE_UDID YOUR_IPAD_UDID
# Omit the UDIDs to select the first available iPhone and iPad.
```

The script boots the phone, seeds the shared HEIC fixture for the system Photos-picker
journey, runs the complete phone suite and an iPad navigation check, snapshots both
profiles, and merges them for the aggregate threshold. The script uses ad hoc simulator signing (`CODE_SIGN_IDENTITY=-`). Do not turn
signing off: the real Keychain test returns `errSecMissingEntitlement` in an
unsigned test host. Generated `.xcresult` bundles and JSON coverage reports go
under the ignored `ios/TestResults/` directory.

`ios/Scripts/coverage.py` retains the `xccov` report and checks production Swift
with Xcode's `llvm-cov` and recorded simulator counters. The installed Xcode
omits local Swift package files from its `xccov` report despite instrumenting
their binaries, so the script includes the app and embedded package frameworks
explicitly. It fails if any production Swift file is missing or aggregate
executable-line coverage is below 95%. It does not remove native adapters,
camera code, UI, or failure branches from measurement. See the dated results in
[implementation status](ios-implementation.md).

For a test-only workspace, use launch arguments in a Debug scheme:

- `--ui-testing`: ephemeral database and deterministic public transport. The UI tests can supply base64 response bodies and statuses keyed by lowercase URL path in the `PINVOYAGER_TEST_RESPONSES` JSON launch environment; this adapter exists only in Debug builds.
- Add `--signed-in-fixture` for a synthetic previously verified account.
- Add `--populated-fixture` for a local voyage and synthetic JPEG.
- Add `--offline-fixture` for offline authoring and publication-review checks.

Release builds do not include these fixtures or recognize these arguments.
They do not establish authentication against a real server.

## Storage and background behavior

The app stores version-one Core Data SQLite records beneath Application Support
`PinVoyager/workspace.sqlite`. Each entity has an immutable ID, account namespace,
and Codable payload; separate entities hold voyages, photos, outbox operations,
working copies, publications, transfers, accounts, and preferences. A private
Core Data context serializes complete transactions. Binary files live under
`PinVoyager/files/{account-id}/`.

`FileVault` excludes private files from automatic backup and uses protection
until the first device unlock. Session credentials use
`AfterFirstUnlockThisDeviceOnly` Keychain items. Originals remain on the device
until explicit account cleanup; uncertain uploads do not cause source removal.
Export includes private metadata and retained originals, with exact-location
wording in the UI. Export directories are not backups created automatically.

The background identifier is `com.pinvoyager.app.sync`. Upload session identifiers
include account ID and generation. The app persists multipart bodies and transfer
records before resuming system tasks. Network-policy changes advance the
generation and cancel outstanding tasks so old callbacks cannot resume private
work in the new generation. Transfers reconcile server receipts before retrying;
HTTP 201 or byte progress alone does not mark a photo accepted.

The system controls scheduling. Force-quit cancels background transfers until
the person reopens the app. Background processing expiration stops scheduling
new work. Photos picker temporary files are copied before they are marked saved;
after restart, `ImportRecovery` reattaches complete originals and rebuilds previews while preserving upload keys and recorded public-upload consent. Missing or damaged acquisitions remain visible needs-source records.

## Physical-device acceptance still required

The bundled `PrivacyInfo.xcprivacy` declares user identifiers, photos, locations,
and voyage/report text for app functionality, with no tracking. It declares
UserDefaults and checking disk capacity before writes. These are implementation
declarations and still need reconciliation with the final deployed backend and
App Store disclosures. See Apple's [required-reason API documentation](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitype)
and [data-collection manifest reference](https://developer.apple.com/documentation/technotes/tn3184-adding-data-collection-details-to-your-privacy-manifest).

Use a test deployment implementing the reviewed native protocol. Exercise real
provider handoff/linking, Keychain access after reboot and unlock, Photos/iCloud
representation and GPS choices, HEIC/WebP display, camera permission/flash and
optional location, dropped upload responses, background relaunch, force-quit,
policy changes, disk exhaustion, universal links, and large photo batches without
a debugger attached. Check VoiceOver, maximum Dynamic Type, Reduce Motion and
iPad navigation. Simulator tests do not establish these system behaviors.

## Authorization and transfer recovery

`AppModel.publicAccess` identifies the current account generation, foreground validation,
network availability, and blocked-account set. Remote detail, profile, map, gallery,
and image views stop displaying a prior response when this value changes, cancel
obsolete view tasks, and reject late responses. An open remote gallery closes when
its authorization context changes. `APIClient.image` shares simultaneous reads only
within an account and credential, allows at most four image requests at once, and
revalidates every subsequent read. No ordinary image response becomes an offline grant.

`BackgroundTransferRecovery.restore` configures each saved URLSession with its persisted
network policy, validates task identities against the outbox and account generation,
and cancels unknown, locked, stale, or removed-photo tasks before resuming valid work.
System upload completion includes task identity, HTTP status, request reference,
Retry-After, response body, and transport error. Recovery saves the shared retry or
attention classification before the application acknowledges background events.
Duplicate completion callbacks and replaced tasks cannot count another retry. A 401
advances the generation, pauses remote work, and cancels the old session.

`SyncCoordinator` polls armed publication intents independently, honors their backoff,
and continues unrelated voyages after a polling failure. Cleanup operations do not
wait for failed photo uploads or Wi-Fi. Canceling a publication whose response was
lost retains a cancellation dependent on resolving that publication's server identity.
An unpublication receipt establishes its historical revision; a separate current
visibility check determines the public/private label, so later website publication
cannot be hidden by an old receipt.

Owner and public galleries open with a full-screen cover that fills the current
app window, including on iPad. Opening from a map preserves that marker's photo
membership. Close returns to the originating voyage. The gallery-size checks in
`PublicBrowsingTests` and `LocalVoyageFlowTests` cover opening from both photos and maps.

The remote gallery offers an ordered thumbnail strip and requests the full processed
variant only after zooming. `ZoomablePhoto.Coordinator` requests this upgrade once per
view; the display image remains visible while it loads. `LocalRepository.undoPhotoRemoval`
restores a retained photo only while the removal request has not been submitted. The
local voyage shows Undo during that interval; a submitted removal must reconcile first.

`PhotoImportService` creates a complete batch of persisted placeholders before asking
providers for bytes, imports one file at a time, continues after an individual provider
failure, and stops remaining acquisitions on request. `PhotoAcquiring` supplies owned
temporary files to this shared service; camera imports use the same saving path.
Starting acquisition invalidates an unsubmitted publication review immediately.

`PhotoRecoveryView` lets the owner choose a replacement or explicitly create a smaller
JPEG. The service assigns fresh photo/upload identities, keeps the old photo until
its replacement is durable, then queues the old upload's cancellation/removal and a
complete ordering successor. Reduced copies retain recorded GPS/capture date. Original
bytes remain available in exports, including retained sources of removed/replaced photos.
A JPEG that would not be smaller returns a field error and keeps the current source.
Interrupted replacement records use the same `ImportRecovery` path after restart.

Owner downloads initially save only verified thumbnail and display variants.
`OwnerDownloads.downloadFull` obtains and revalidates a fresh manifest before saving a
full variant requested by zoom. Confirmed website removal clears the corresponding
download certificates and files; imported sources remain recoverable. A network failure
or expired session does not establish deletion. An expired session pauses synchronization
while allowing previously verified local authoring; sign-out locks that workspace.

Background response evidence lives inside the same account file namespace as sources.
It is bounded to 1 MiB per response, includes status/request reference/Retry-After and
transport outcome, replays before task adoption on launch, and is removed after the
outbox checkpoint succeeds. Purging account files also removes this evidence. A task
creation/adoption rechecks that its photo still exists before resume; removal during
creation cancels that task without canceling sibling transfers.

Photo order/removal conflicts use `APIClient.ownerSnapshot` to read the owner-visible
voyage, paginate exact owner metadata, and check the voyage revision again. A changed
revision or incomplete membership requires another review. `ConflictResolutionView`
compares both ordered lists and supports website order plus local additions, an
explicit combined order, or keeping website membership. `LocalRepository.resolvePhotoConflict`
retains the rejected request, cancels superseded unsent changes, and queues successors
against the reviewed revision. It refuses to resolve while another submitted outcome
is uncertain. Website-deleted photos remain removed; their retained sources can be
explicitly added again with `addAgain`, which creates new photo and upload identities.

`ForegroundPolling.observe` resets its 2/5/15/30-second delay when the processing
snapshot changes. Both local and remote voyage screens stop polling when processing
finishes or the view task is canceled. `PollingTests` checks the delay reset and stop
conditions; `OwnerSnapshotTests` and `WorkflowTests` check conflict membership,
private metadata, immutable requests, and new identities.

Help opens bundled Privacy and Terms text without requiring a connection. The bundle
preserves the website drafts' revision, review status, operator/contact information,
and template attribution; it does not assign an effective date. Regenerate it with
`python3 ios/Scripts/bundle-legal.py` after editing the website policy sources, and run
`python3 ios/Scripts/bundle-legal.py --check` to detect drift. `BundledLegal` reads the
small archive off the main thread. `LegalResourceTests` checks its content and missing
resource error, and `AccountFlowTests` exercises both documents with the app offline.

Publication notifications default off. `PublicationNotifier` asks for system permission
only when the setting is enabled, waits for a published intent and confirmed current
visibility, and persists a per-intent checkpoint. A stable system identifier recovers
notifications scheduled before a checkpoint failure. Concurrent checks share one pass;
a generation/access change during delivery removes the stale notification. Delivery
failures preserve publication and allow another check. `SystemPublicationNotifications`
uses UserNotifications for immediate local delivery, not a server push service. A server
completion while iOS cannot run may be learned only on a later foreground visit.
`PublicationNotificationTests` and `AppModelTests` cover preference, state, permission,
retry, restart, duplicate-check, and sign-out behavior.

`PhotoImportPresentation` owns the import screen's progress, retained acquisition
errors, and stop action. It ignores empty selections and a second batch while an
import is active. `PhotoImportPresentationTests` verifies durable success, provider
failure, explicit public-upload consent, and stopping after the current copy while
keeping remaining selections recoverable. The SwiftUI screen composes this state
with the system Photos picker and the shared `PhotoImportService`.

The native job in `.github/workflows/checks.yml` checks Swift formatting and bundled
policy drift, runs the complete Simulator suite with the 95% aggregate Swift line
requirement, builds Release for Simulator, and retains `.xcresult`/coverage artifacts.
It selects the macOS 26 image and Xcode 26.6 documented in the
[GitHub runner image inventory](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-Readme.md).
The hosted job has not been run from this workspace; local results remain the
verification evidence until that job executes on a pushed branch.

Universal links use the `applinks:pinvoyager.com` entitlement and the trusted web
origin. `ios/Scripts/generate_association.py` generates the HTTPS association body
from the signed app's `application-identifier` entitlement (App ID prefix plus
`com.pinvoyager.app`). Pass that value with `--application-identifier` and a local
`--output` path. The generator includes voyage, profile, upload and settings routes;
auth callbacks and arbitrary external destinations remain outside those rules.
`test_association.py` checks supported paths and rejects unexpanded or unrelated
identifiers. Host the reviewed output at
`https://pinvoyager.com/.well-known/apple-app-site-association` as JSON without a
redirect when configuring the signed app. The actual App ID prefix and hosting
configuration are still required; this work did not deploy an association document.

`ios/Scripts/test.sh` runs the full iPhone suite and the iPad guest-navigation check,
then merges their LLVM profiles before measuring every production Swift file. Its
optional first and second arguments select the iPhone and iPad simulator IDs. Each
`.xcresult` has a sibling `.profdata` snapshot so later test runs cannot overwrite its
counters. `coverage.py` accepts repeated `--profile` or `--device` arguments; merge
profiles collected from the same production sources and build. The regression check
showed the previous single-device parser discarded iPhone coverage when passed two
IDs; the combined report preserves the phone counts and adds the iPad layout paths.
A failed test or coverage percentage still makes the script fail.
