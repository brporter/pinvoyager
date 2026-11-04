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

The account header and sign-in sheet in `ios/PinVoyager/Features/AccountView.swift`
use the `BrandIcon` image set in `ios/PinVoyager/DesignSystem/Assets.xcassets`.
It contains copies of the default and dark `AppIcon` artwork, selected by the
current appearance. Keep these copies aligned when replacing the app icon;
the `AppIcon` app-icon set is separate from images loaded inside the app.
`ios/PinVoyagerTests/BrandAssetTests.swift::testAccountBrandIconLoadsDistinctLightAndDarkArtwork`
checks that both appearances load distinct artwork from the built app bundle.

## Swift formatting

The app, local package, and tests follow the formatting conventions in the
[Google Swift style guide](https://google.github.io/swift/).
`ios/.swift-format` configures the bundled formatter for two-space indentation,
100-character lines, one argument or generic requirement per line when a list
wraps, separate parameterized attributes, grouped and sorted imports (including
conditional imports), and trailing commas in multiline collections. Existing
line breaks and single blank lines are preserved; repeated blank lines collapse
to one. Multiline string contents are not reflowed.

From the repository root, format all maintained Swift files:

```sh
swift format --in-place --configuration ios/.swift-format --recursive \
  ios/Package.swift ios/Sources ios/Tests ios/PinVoyager \
  ios/PinVoyagerTests ios/PinVoyagerUITests
```

Run the same checks as `.github/workflows/checks.yml`:

```sh
python3 -m unittest discover -s ios/Scripts -p 'test_format.py'
swift format lint --strict --configuration ios/.swift-format --recursive \
  ios/Package.swift ios/Sources ios/Tests ios/PinVoyager \
  ios/PinVoyagerTests ios/PinVoyagerUITests
```

`ios/Scripts/test_format.py::SwiftFormatTests` checks wrapping, attributes,
import grouping, retained member spacing, indentation, and collection commas.
CI runs these examples and rejects formatting differences using the explicit
configuration, including in `Package.swift`.

Some Google conventions still require review. Put one blank line between
methods, initializers, computed properties, nested types, and other members;
adjacent single-line stored properties or enum cases may stay together. Use
blank lines within a function to separate logical steps. The formatter retains
these separators but cannot insert or require them. Keep each element on its
own line in a multiline array, dictionary, or inheritance list; the formatter
preserves this layout but can otherwise pack several elements onto a line.
It also cannot fully enforce Google's continuation alignment, attribute ordering,
declaration organization, or naming and API-design guidance. Existing SwiftUI multiple-trailing-closure
syntax remains in use; changing call structure is outside this formatting pass.
Review these items when editing code instead of treating a successful lint as
proof that every recommendation in the guide has been satisfied.

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

GitHub Actions temporarily runs only the non-UI Swift tests: the Swift package
and `PinVoyagerTests` application-model/adapter tests. The workflow sets
`IOS_UI_TESTS=0` when invoking `ios/Scripts/test.sh`, which passes
`-skip-testing:PinVoyagerUITests`, runs on one phone simulator without cloning or
seeding Photos, and skips the iPad UI check. CI continues formatting, script
checks, the Release simulator build, and test/coverage artifact uploads.

Reproduce the CI test selection locally:

```sh
IOS_UI_TESTS=0 ios/Scripts/test.sh YOUR_IPHONE_UDID
```

This mode generates coverage reports for every production Swift file, with
`threshold_enforced: false` in the Swift summary. Its reduced coverage does not
enforce the app-wide 95% requirement because UI journeys are deliberately absent.
No production files are excluded. The full local suite still enforces 95%; web
and API coverage checks are unchanged. `test_runner.py::TestRunnerTests` verifies
that CI mode skips the UI target, avoids simulator cloning/media seeding and iPad
discovery, and uses only newly generated coverage counters. The local CI-mode
validation passed all 258 non-UI tests with zero skipped tests
(`ios/TestResults/20261102-162721.xcresult`). Its measured app-wide Swift coverage
was 8,709 / 19,672 lines (44.27%); report-only mode succeeded, while the default
95% coverage check rejected the same profiles as expected. The new runner
regression failed before CI-mode support and passed afterward; all 12 Python
script tests passed.

`test_coverage.py::CoverageProfilesTests.test_help_formats_the_report_only_option_without_requiring_xcode`
invokes the real coverage command with `--help`. It first reproduced the invalid
percent formatting in the `--report-only` help string, then passed after escaping
`95%%` for argparse (displayed as `95%`). Python 3.15 on the hosted runner rejects
invalid help strings while registering arguments; local Python 3.9 otherwise
reaches this validation only when formatting help. All 13 script tests pass with
this regression included.

Run the complete suite locally, including all UI tests and the 95%
executable-line coverage check:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
ios/Scripts/test.sh YOUR_IPHONE_UDID YOUR_IPAD_UDID
# Omit the UDIDs to select the first available iPhone and iPad.
```

The script creates and erases a disposable clone of the selected phone, then boots
it and seeds the shared HEIC fixture before Xcode creates its worker clones.
This gives notification authorization a clean initial state without erasing the
selected device. An EXIT trap shuts down and deletes the disposable phone after
success or failure. It runs the complete phone suite with two
UI-test workers, followed by an iPad navigation check. Set `IOS_TEST_WORKERS=1`
to limit the phone run to one worker. Each worker uses a separate simulator;
the script never runs two UI drivers against the same device.

Before each phone or iPad run, the script removes the generated
`ios/.derivedData/Build/ProfileData` counters. Afterward, it snapshots every
worker profile into a `*-profiles` directory beside the result bundle.
Xcode 27.1 merges its workers under the requested simulator's UUID; the collector
also accepts separate worker directories rather than assuming a single profile.
`coverage.py --profile-directory` merges these snapshots from the same build,
including the iPad counters, and rejects missing or empty snapshot directories.
`ios/Scripts/test_coverage.py::CoverageProfilesTests` checks worker collection,
stale-profile isolation, and compatibility with explicit profile arguments.

The script uses ad hoc simulator signing (`CODE_SIGN_IDENTITY=-`). Do not turn
signing off: the real Keychain test returns `errSecMissingEntitlement` in an
unsigned test host. Generated `.xcresult` bundles and JSON coverage reports go
under the ignored `ios/TestResults/` directory.

`AccountFlowTests.testStorageControlsRequireCleanupConsentAndOfferVoyageExportsAndDownloads`
waits up to 30 seconds for the system Files export picker's Cancel control. The
picker can take longer than five seconds to become accessible on a fresh CI simulator.

`ios/PinVoyagerUITests/AccessibilityActions.swift::XCUIApplication.launchFixture`
and `FixtureLaunch.launch(tab:arguments:)` set `PINVOYAGER_TEST_TAB` to the tab
needed by each test. `RootView` calls
`DevelopmentFixtures.restoreNavigation` only in Debug UI testing. A valid
starting tab skips saved scene routes; launches without that setting still use
`SceneRouter.restore`. This prevents earlier tests' selected tabs and nested
public destinations from determining the next test's starting screen. Ordinary
app launches and Release behavior retain scene restoration.
`BootstrapTests.testFixtureStartingTabsIgnoreSavedDestinationsButOrdinaryLaunchesRestoreThem`
checks all four starting tabs and the restoration fallback.
`PublicBrowsingTests.testDiscoverSetupReturnsToRecentFromEveryTabAndANestedProfile`
checks tab switching, returning from nested destinations, and a fixture relaunch.
`showDiscover` is used by that navigation regression, rather than repeated in
unrelated browsing, profile, and reporting setup.

`XCUIElement.waitUntilExists` first checks `exists`; it retains XCTest's polling
wait and the original timeout if the element is absent. System pickers and
asynchronous transitions still wait for their controls. Editor save/discard and
precision-picker tests wait for their sheet's navigation bar to disappear
before reopening it;
an element behind a dismissing sheet is not evidence that dismissal finished.
The publication editor test dismisses the keyboard before scrolling to Save.
`XCUIElement.reveal` reads the container frame once per call and stops after its
swipe limit or three
swipes that leave a known target at the same position. It reports failure at the
call site. If the keyboard is open, it keeps drags above the keyboard and updates
the visible area if the keyboard dismisses. The invalid recovered draft test
queries the editor's
`voyage.description-count` identifier instead of matching every static text's
label suffix. `UITestCase` stops a case after its first failure so subsequent
actions do not run on an unexpected screen.

The discovery setup and scroll-position regressions use `waitUntilHittable` on
the voyage's navigation button rather than its static-text child. The system
Photos picker helper selects the center of a matched photo after checking that
its frame is nonempty and inside the window. On the local iOS 26.4.1 simulator,
XCTest reported no hit point for a visibly unobstructed photo even after waiting;
the center tap completed selection and the test's existing import assertions
passed. These helpers use UI state and geometry, with no OS-version conditions
or fixed sleeps.

`AccessibilityActions.selectFirstSystemPhoto` allows up to 30 seconds for a photo
to appear in the system picker. A timeout attaches a screenshot, the accessibility
hierarchy, and elapsed time. The wait returns immediately when a photo exists.
`PhotoPickerTests.testSystemPickerCopiesTheSelectedPhotoAndKeepsItsBatchCellularPermission`
waits for the picker to close, the cellular switch to be enabled and reset, and
Close to be enabled. A saved photo row can appear while the batch is still being
prepared; it does not establish batch completion.

`NotificationFlowTests` enables the Debug-only
`--notification-permission-diagnostics` launch argument and requires the initial
system authorization status to be `notDetermined`. It requires Allow to appear
within 30 seconds, then waits up to 15 seconds for the persisted opt-in to appear
in the switch before checking real foreground notification delivery. The diagnostic
label is omitted from ordinary launches and Release builds. `SystemNotificationCenter`
logs request start, completion, authorization status, and errors under the
`NotificationPermission` category. `PublicationNotificationAdapterTests.testDelayedPermissionUpdatesTheVisiblePreferenceOnlyAfterAuthorizationCompletes`
holds the permission response pending and verifies that the model and saved policy
change only after authorization completes.

For this CI-failure follow-up, the notification UI test first failed because the
initial-authorization diagnostic was absent (`permission-red-2.xcresult`). After
implementation, `ios/Scripts/test.sh` passed all 314 phone tests (258 non-UI and
56 UI), the iPad navigation test, and aggregate production Swift line coverage
at 18,799 / 19,672 lines (95.56%). Results are in the ignored
`ios/TestResults/20261102-152009.xcresult` and matching `-ipad.xcresult` bundles.
The Release simulator build, Swift formatting, all 11 Python script tests,
bundled-policy check, shell syntax, and `git diff --check` also passed. API and
web source were unchanged; their suites and coverage were not rerun for this
native-test follow-up. The three affected UI scenarios also passed on a freshly
erased iOS 26.4.1 simulator (`ios/TestResults/ci-fixes-ios2641.xcresult`).

`PinVoyager.xctestplan` enables execution timeouts: 120 seconds per test by
default, with a maximum allowance of 180 seconds. These bounds help preserve a
completed result bundle when an accessibility query stalls; they do not make
healthy tests faster. The separate GitHub Actions job timeout remains 45 minutes.

On November 2, 2026, the same 56 UI scenarios passed before and after these changes
on Xcode 27.1 with iOS 27.0 iPhone 18 Pro simulators. Elapsed UI time is measured
from the first case's start to the last case's finish in the result bundles,
including gaps between cases and overlapping workers:

| Configuration | UI elapsed time | Change from baseline |
| --- | --- | --- |
| Original tests, serial | 1,191.5 seconds (19m 51s) | Baseline |
| Initial optimization check, serial | 804.1 seconds (13m 24s) | 32.5% less |
| Final implementation, two workers | 413.8 seconds (6m 54s) | 65.3% less |

The initial serial check preceded the older-OS selector and keyboard refinements.
It included an additional fixture relaunch assertion and made 19 polling waits
instead of 248. No UI scenario was removed. The complete
two-worker `ios/Scripts/test.sh` run, including the incremental build, 257 non-UI
tests, 56 iPhone UI tests, the iPad navigation check, and coverage processing,
took 484.9 seconds (8m 05s). Production Swift coverage was 18,722 / 19,588 executable
lines (95.58%). These are local measurements; a fresh hosted build and simulator
may have different timings.

The regression for fixture starting tabs first failed by restoring Account and
old destinations, then passed after implementing `restoreNavigation`. The editor
regression first failed to find `voyage.description-count`, then passed after
adding that identifier. The three coverage-collection tests passed after first
demonstrating missing worker collection and acceptance of empty snapshots.
Faster execution exposed an editor dismissal race and keyboard-obstructed
scrolling; both corrected tests passed twice consecutively before the full runs.

Focused compatibility checks also ran on the iOS 26.4.1 iPhone 17 Pro simulator
(build 23E254a). Eleven of twelve cases passed together; the remaining
scroll-position case passed three consecutive runs after its query was changed
to the navigation button. The selection and scrolling corrections described
above address the failures found during this check. This was a focused older-OS
check, not the complete suite on every supported iOS version. The Release
simulator build, all 11 Python script tests, bundled-policy validation, Swift
formatting, and shell syntax checks passed. API and web checks were not rerun;
their implementation and tests were unchanged.

The local artifacts are under the ignored `ios/TestResults/ui-performance/`
directory (`baseline.xcresult`, `serial.xcresult`, logs, and timing JSON).
The complete parallel run is `ios/TestResults/20261102-132338.xcresult`, with
the iPad bundle, profile snapshots, and aggregate coverage JSON beside it.
For another serial UI measurement, use `xcodebuild test` with the project's
PinVoyager scheme and test plan, `-only-testing:PinVoyagerUITests`,
`-parallel-testing-enabled NO`, and a unique `-resultBundlePath`. Use
`ios/Scripts/test.sh` for the complete two-worker and coverage check.

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
- `PINVOYAGER_TEST_TAB`: optional launch environment value `discover`, `voyages`, `sync`, or `account` to start a Debug UI fixture at that tab without restoring old routes. Omit it when testing scene restoration.
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
It selects the `xcode-27` runner and Xcode 27.1 documented in the
[GitHub runner image inventory](https://github.com/actions/runner-images/blob/main/images/macos/xcode-27-arm64-Readme.md).
That image includes iOS 27 simulators. `ios/Scripts/test.sh::choose_device` selects
the first available iPhone and iPad when simulator IDs are not supplied.
Inspect the `checks / ios` job for the pushed commit to verify hosted results.
Older iOS runtime compatibility still requires separate checks.

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

## Native Apple sign-in setup

Enable Sign in with Apple for the signed app's App ID. The project includes the
`com.apple.developer.applesignin` entitlement; regenerate provisioning profiles if
needed. Set `PINVOYAGER__AUTH__PROVIDERS__APPLE__NATIVE_CLIENT_ID` to the exact bundle ID
in addition to the existing Services ID, team ID, key ID, and signing key settings.
Associate the web Services ID with this primary App ID in Apple's portal before
signing in an existing website user. Verify both clients receive the same Apple
subject and retain the same PinVoyager account. The owner verified on November 4,
2026 that production Services ID `com.pinvoyager` is associated with App ID
`com.pinvoyager.app`. See [production deployment](deployment.md#apple) for the exact
environment settings and API recreation command. Never configure the Services ID as
the native audience. Production uses Apple's fixed issuer and HTTPS API origin.

A server without a native client ID still supports web Apple sign-in, but advertises
`native_apple_authentication: false`; iOS disables Apple sign-in on that server.
The native app fetches a nonce before showing Apple's system sheet. The custom Apple
entry button uses official artwork supplied by Apple Design Resources; see
[provider button guidelines](provider-sign-in-buttons.md) for provenance and layout. Google and
Microsoft still use `ASWebAuthenticationSession`; the account-switch toggle affects
only those browser sessions. See [native protocol](native-api.md#native-apple-authorization).

Provider artwork comes from [Google's approved assets](https://developers.google.com/identity/branding-guidelines)
and [Microsoft's sign-in assets](https://learn.microsoft.com/en-us/entra/identity-platform/howto-add-branding-in-apps).
The bundled Google glyph preserves the supplied artwork at 3x resolution, extracted
from the center of Google's iOS icon button. Microsoft's SVG is unchanged. Google
Sans is bundled under the SIL Open Font License; see `ios/PinVoyager/Resources/Fonts`.
