# Native API behavior

The implementation in this repository is under active development. Capability
advertising enables native authentication separately from synchronization and now
advertises native synchronization protocol version 1. This document
records implemented behavior; [the iOS design](ios-app-design.md) records the full target.

## Authentication

`http/auth/native.rs::{create_attempt,start,complete_verified,exchange,logout}` wraps
existing provider verification in a ten-minute, one-use native attempt. The browser
bootstrap retains provider state, nonce, provider PKCE, and the browser-binding
cookie. A verified callback returns an allowlisted `pinvoyager://auth/callback`
handoff with app state, never a session. The sixty-second handoff is hashed in
PostgreSQL and bound to the app S256 challenge, attempt, mode, callback, and account.
Exchange consumes it and creates a native session atomically. A lost exchange
response requires fresh authentication. Identity resolution is shared with browser
sign-in (`auth/sign_in.rs::resolve_identity`); provider plus subject identifies an
account, never email. Linking binds the initiating account in the attempt.

`auth/extract.rs::resolve` accepts browser cookies or native bearer credentials.
`native_sessions` distinguishes the transports. Mixed credentials, multiple bearer
headers, malformed/invalid native credentials, and browser tokens presented as
bearer tokens fail with 401. Native logout is idempotent and cannot revoke a browser
session. Handoff/exchange/capability responses use `no-store`; URLs and logs never
carry session tokens. Provider tokens are not retained.

`crates/api/tests/native_auth.rs` drives the real router against the loopback mock
OIDC provider, including single-use exchange, wrong verifier/attempt, expiry,
callback/binding checks, and transport isolation. Its initial run returned 404 for
the missing routes; all four initial cases passed after implementation. Existing
browser authentication suites also passed.

## Conditional mutations and receipts

Migrations `20261025000002_native_revisions_and_receipts.sql` and
`20261025000003_publication_intents.sql` add content revisions and transactional
invalidation. Metadata updates (including explicit unchanged unpublication), photo
membership and ordering advance the revision. Image-conversion status does not.
Owner detail adds a decimal-string `revision`; other detail responses omit it.
Public positions still apply the voyage precision.

`http/mutations.rs::run` requires native UUID `Idempotency-Key` values and strong
`If-Match: "v:17"` preconditions for overwrites/removals. It hashes method, route,
precondition, and canonical payload. Under account/resource locks, identical replays
return the recorded outcome before checking the current revision; key reuse with
changed input returns 409. A stale revision returns 412 `revision_conflict`.
Permanent rejection receipts commit while savepoints roll back attempted changes.
Server failures remain uncertain and can be retried after lookup. Operation receipt
lookup is account-only and returns `not_seen` for unknown keys. Compact receipts
survive voyage/photo deletion and cannot recreate deleted destinations.

`http/uploads.rs` maps an external UUID to upload ordinal zero. Upload commit,
cancellation, and photo deletion serialize under the voyage lock. A cancellation
records a tombstone even before acceptance; a later upload cannot resurrect it.
Native uploads require one file, a stable key, and explicit draft/public visibility.
An accepted replay returns the original acceptance even if visibility changed;
new bytes or a new visibility mismatch fail before commit. PUT `photo-order`
requires complete membership without duplicate/foreign/missing IDs, up to 10,000.
The website's partial reorder route remains supported and advances revisions.

`crates/api/tests/native_sync.rs` started red with missing routes/contracts; its four
initial integration cases pass for creation/replay/deletion, stale revisions,
upload cancellation/tombstones, visibility, and full ordering. Rust-captured owner
fixtures and `web/src/api/types.ts` include the optional revision.

## Reviewed publication

`voyages/publication.rs` stores immutable metadata digest, consent expiry, and an
ordered manifest of existing photo IDs or future upload keys/content hashes.
Registration requires an active named account, a private voyage with title and
description, at least one photo, the reviewed revision, and exact membership.
Only one intent may be active per voyage; consent is bounded to seven days.

Expected uploads advance the intent's tracked revision under the same transaction.
Wrong bytes cannot satisfy it. Other uploads, metadata/order/removal changes, and
explicit unpublication invalidate active consent. Conversion completion and periodic
worker maintenance call the same finalizer. Under account/voyage locks it checks
account status, expiry, metadata, revision, membership and every variant, then applies
the frozen order and publishes atomically. Failed or deleted photos require review;
no subset is published. Cancellation addresses a unique intent and tolerates content
revision changes; it returns already-published when finalization wins and never
unpublishes. GET status reconciles recovery before returning current visibility.

Historical operation receipts include voyage identity. The native reader revalidates
current voyage visibility before displaying historical publication as public;
publication-status responses include current `published` separately from intent status.

`crates/api/tests/native_publication.rs` began with expected missing-route failures.
Six cases now pass, exercising actual JPEG conversion, out-of-order expected uploads,
unrelated changes, wrong bytes, cancellation/expiry/failure/suspension, explicit web
unpublication, missing display names, and damaged accepted receipts. The Swift test
`aHistoricalPublicationReceiptRevalidatesCurrentVisibility` began red for the missing
confirmation state and passes with current visibility reconciliation.

## Upload deadlines

`http/middleware.rs::timeout_request` preserves the ordinary 30-second request
limit and gives POST voyage-photo uploads a separate 900-second deadline. Tower's
`TimeoutBody` enforces a 90-second incoming-frame inactivity timeout, independent
of validation/conversion work. `http/photos.rs::ingest_part` reserves conversion
capacity before opening a transaction, for at most ten seconds. Queue saturation
returns 503 with `Retry-After: 10`, without inserting a photo. Upload inactivity
returns 408 with retry guidance. Configure these through
`server.upload_request_timeout_secs`, `server.upload_body_idle_timeout_secs`, and
`photos.variant_queue_wait_secs`, or corresponding `PINVOYAGER__` variables.

`crates/api/tests/native_upload_timeouts.rs` started with missing configuration
failures. Its three cases pass for slow streaming versus ordinary deadlines,
full-queue rejection without commit, and body inactivity. Reverse-proxy and physical
cellular behavior still need deployment/device acceptance; multipart retries restart
whole files and do not resume at a byte offset.

## Owner downloads

`http/downloads.rs` adds owner-only offline manifests and explicit variant transfers.
A manifest snapshots the voyage under a shared lock and includes revision, ordered
owner metadata, and the SHA-256 and byte count of each stripped WebP variant. Hashing
runs on the blocking pool. Pending processing returns 409; missing files return 404.
Each transfer rechecks ownership and emits `X-PinVoyager-Retention:
owner_offline_copy_v1`. Originals have no download route. Ordinary public image
cache policy is unchanged.

`ios/Sources/PinVoyagerData/OwnerDownloads.swift::OwnerDownloads` checks permission,
account, destination, complete variant membership, paths, hashes, and sizes. It stages
protected files, reuses verified partial files, fetches the manifest again, and certifies
copies in one workspace transaction only if the snapshot and account generation agree.
Downloads retain metadata and processed variants; they do not replace imported originals
or acquire offline basemaps. Local owner viewing uses certified copies while offline.
Removing a download removes its certificate before deleting files.

`native_downloads.rs` first failed on the missing route, then passed against the real
JPEG conversion pipeline. `DownloadTests.swift` first failed on missing download types,
then passed for SQLite reopening, exact file verification, removal, corruption, and
changed snapshots. App model tests first failed on missing commands, then passed with
connection requirements and local-work preservation.

## Reports, blocking, and moderation

`http/moderation.rs` accepts signed-in idempotent reports for public voyages/photos
and travelers. Reports contain at most a 100-character reason and 2,000-character
detail; a serialized database check limits each reporter to twenty submissions per
rolling day (429 with Retry-After; no permanent mutation receipt). Reports and moderator notes are visible only to active administrators.
The paged administrator queue, detail, and action routes require a current role check.
Review actions require a note. Holding a voyage unpublishes it, invalidates armed
publication consent, and prevents direct or reviewed republication. A traveler hold
applies to that traveler's existing voyages; suspension is the separate administrator
action that prevents future authoring. Releasing a hold never republishes content.

Blocks are account-scoped. GET returns the current blocked IDs; PUT/DELETE use durable
mutation receipts. Discovery filters blocked owners in SQL before feed pagination and
before map candidate limits/grouping. Blocked profiles and direct voyage/map/photo
reads return 404 for that account. Owners retain access to their own held drafts.
Anonymous public visibility remains governed by publication and moderation status.

`native_moderation.rs` began with expected missing-route failures. Both initial cases
pass for receipt replay, filtering before pagination, profile/detail/map access,
unblocking, private moderator access, hold/release, and publication eligibility.
The web moderation interface and expanded failure/race coverage remain in progress.

## Website concurrency

`web/src/api/client.ts::request` sends the reviewed strong revision when supplied.
`VoyageEdit.tsx::EditForm` retains its initial revision across background query refreshes
and advances it only after an acknowledged save. A 412 preserves input, disables further
save/publication/deletion, and offers an explicit reload that discards those edits.
The initial client/editor tests failed for missing headers/conflict controls; the client,
hook, and editor suites passed after implementation (75 tests).

## Account deletion and recovery

`auth/provider.rs::AuthProvider::exchange_for_deletion` and the existing verified
provider callback implement same-account, deletion-only reauthentication. Ordinary
sign-in/link callbacks cannot produce deletion proof. An attempt expires after ten
minutes; a proof is single-use, hashed, bound to the current linked provider subject,
and expires after ten minutes. Reauthentication issues proof rather than another
application session. Wrong subjects cannot create or link an account through this path.

Apple's override (`auth/providers/apple.rs`, `OidcCore::exchange_with_revocation`)
verifies the expected subject before immediately revoking a freshly exchanged refresh
token, or the access token if no refresh token is returned. Neither token leaves the
provider module or is persisted. Revocation must return 200 before proof is issued.
This follows [Apple's token revocation contract](https://developer.apple.com/documentation/signinwithapplerestapi/revoke-tokens).
Preparing deletion requires a matching Apple revocation proof whenever Apple is
linked, including when the primary reauthentication used another provider. The app
chooses linked Apple verification directly in this case.

`http/account_deletions.rs` prepares a ten-minute request without deleting data.
The client generates and saves a random status capability **before preparation**,
then sends it with the proof and stable UUID operation key. This refines the proposed
server-generated capability: the server stores only its hash, and a lost preparation
response cannot lose the only recovery credential. Identical preparation replays return
the same receipt; changed input conflicts. Confirmation requires the same account,
unexpired preparation, and unchanged Apple proof binding. The last active administrator
must assign another administrator first.

Confirmation and `accounts/deletion.rs::remove` lock account, voyages, and sorted
content hashes, remove account/public access and all sessions/identities/intents,
reserve former profile names, and queue existing durable file cleanup in one transaction.
Administrator deletion uses the same removal service. A request with file work reports
`deleting` until maintenance confirms cleanup; only then is it `complete`. Shared bytes
remain when another account still references them. The independent `Deletion` credential
can read only progress after session revocation. Minimal outcomes remain for thirty days.

`AccountDeletionService` saves recovery records in a separate Keychain service using
the same typed `KeychainRecords` adapter as session storage. Preparation and confirmation
have separate commands. Uncertain confirmation retains its capability and cannot be
overwritten by a new preparation. Recovery does not need a session. The native view
provides export, linked-provider verification, explicit destructive confirmation, and
status lookup. Confirmed removal locks retained local work and cancels account transfers;
it does not silently discard unsynchronized originals.

`native_auth.rs::deletion_requires_recent_purpose_proof_and_a_persisted_status_capability`
began red on missing routes and passed through the loopback OIDC provider. The Apple
provider test began red on unavailable revocation and passed for same-subject checks,
request parameters, and failed revocation. Native deletion tests began red for missing
service/recovery interfaces and passed for proof/session separation, durable uncertain
preparation, and session-independent status. App model tests passed for locking the
workspace after confirmed removal. Real Apple/provider and physical-device acceptance
remain deployment checks, not claimed by these loopback/simulator results.

## Native account routing and independent operations

`AppModel::acceptSession`, `resumePendingRoute`, and `saveDisplayName` preserve the
requested owner destination while authentication or initial name setup is unfinished.
A successful name claim fetches the same-account profile before closing setup and
resuming navigation; a canceled setup leaves browsing available and uploads/publication
paused for unnamed accounts. `AppModelTests` began red for these missing commands and
passed for the complete name/setup/creation path.

`AppModel::activate` serializes foreground account verification, capability refresh,
block reconciliation, synchronization, and public discovery refresh. `reconcileBlocks`
applies server changes while retaining pending local block/unblock choices. Operations
use `OutboxOperation::scope` to order changes to the same voyage or blocked traveler;
a failed report cannot stop unrelated account operations. The repository/coordinator
and app model tests first exposed the missing isolation/reconciliation, then passed.

`native_contract_fixtures.rs`, the shared `support/contract_fixtures.rs` helper, and
`native_auth.rs` capture native capability/session/operation/upload/publication/download
and Unicode digest contracts from the real router. The server capability initially
failed the protocol-version expectation; version 1 now passes. Swift's
`nativeContractsMatchRustCapturedFixtures` and the TypeScript contract assignments
check the captured shapes. The Unicode digest fixture includes quotes, backslashes,
control characters, non-ASCII separators, and emoji and matches Foundation's encoding.

Native photo requests validate the complete one-file multipart body before accepting or
queuing a photo. An additional file or incomplete trailing part returns 400 without a
photo row or accepted upload receipt. Browser multi-file requests retain independent
per-file acceptance. `http/photos.rs::{stage_part,receive_part,accept_staged,ingest_part}`
share staging and ingestion; `tests/native_sync.rs::native_upload_rejects_additional_or_incomplete_parts_before_accepting_any_photo`
checks the formerly partial native acceptance. Conditional legacy ordering and direct
photo deletion also share revisions/receipts, covered by
`conditional_legacy_order_and_direct_photo_deletion_share_revisions_and_receipts`.

`tests/native_downloads.rs::unavailable_variant_files_return_private_errors_and_never_stream_directories`
checks missing files, corrupt directories, and recovery without exposing storage paths.
`AppError::{internal,from_file}` centralize private error mapping; manifest hashing uses
`blocking::run_blocking` to retain tracing and keep blocking work off Tokio workers.

## Revision proof in receipts

`http::mutations::run` stores `previous_revision` alongside the resulting `revision`
for successful requests whose `If-Match` was checked. Upload ingestion records the
voyage revision immediately before inserting the photo while holding the voyage row
lock; upload-key cancellation records the actual current revision, since cancellation
intentionally remains valid after its upload changed the voyage. These responses
include both values as decimal strings. Creation and historical receipts with no
known predecessor omit `previous_revision`; the server does not infer it by subtracting
one. A complete ordering operation can advance several revisions.

`ios/Sources/PinVoyagerData/SyncCoordinator.swift` (`advanceRevision`) persists these
edges on `OutboxOperation` and advances the local metadata baseline only through a
connected chain starting at its loaded revision. Disconnected edges survive restart.
The coordinator reconciles submitted work before preparing new writes. Periodic
publication status changes visibility and intent state without replacing the metadata
baseline. `NativeSyncRemote.receipt` preserves a historical publication receipt's
revision while separately checking current visibility. An intervening website edit
therefore continues to produce a revision conflict instead of silently being overwritten.

Checks: `crates/api/tests/native_sync.rs` covers accepted uploads, direct deletion,
and upload-key cancellation with stale supplied revisions. `SyncTests` covers an
unseen website revision, publication status without a snapshot, and out-of-order
upload receipts across a SQLite restart. `AdapterTests` checks historical publication
revision and current visibility independently. Migration
`20261025000006_receipt_revision_edges.sql` adds nullable fields without rewriting
existing receipts.

Publication cancellation accepts stale content revisions so owners can stop an armed
review without first loading unrelated edits. Its receipt therefore omits a revision
predecessor. `mutations::run` derives a predecessor from `If-Match` only when the route
requires and checks that revision, or when the action explicitly records its actual
starting revision. The native cancellation request does not require a metadata
revision. `native_publication::cancellation_does_not_claim_to_have_checked_a_stale_content_revision`
and `WorkflowTests.cancelingKnownPublicationNeverRequiresAMetadataRevision` cover
this distinction.
