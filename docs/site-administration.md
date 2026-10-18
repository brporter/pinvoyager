# Site administration

Site administrators use `/admin` to list accounts and `/admin/users/:id` to manage
one account. The account menu includes **Site administration** only when
`GET /api/me` reports `user.is_site_admin: true`. The browser checks that role
before fetching administration data; every administration API request independently
checks the current database role. Anonymous requests return 401 and signed-in
non-administrators return 403.

The migration `20261018000001_site_administration.sql` adds `users.is_site_admin`
and `users.suspended`, both defaulting to false. Public profile updates cannot grant
administration access. Initial production grants use the UUID operator tool; later
grants can use the administration UI. Debug-only development sign-in supports an
explicit grant checkbox. Provider sign-in never grants roles from email claims.
Public profile responses do not expose the administration flag.

## Bootstrap

Grant the first administrator to an existing account by UUID using the Rust
operator tool included in the API image. Sign in normally first, then obtain your
user ID from the signed-in `GET /api/me` response. From the production Compose
directory, run:

```sh
docker compose exec api pinvoyager-admin YOUR-ACCOUNT-UUID
```

`pinvoyager-admin UUID` loads the same `config.toml` and `PINVOYAGER__DATABASE__...`
settings as the API. It connects directly to PostgreSQL without starting HTTP,
contacting providers, or running migrations. The API must have migrated the database.
The tool locks the account row through commit so concurrent grants serialize:

- An existing non-administrator receives `is_site_admin = true` and an updated
  `updated_at` timestamp. The tool reports the grant and exits 0.
- An existing administrator produces an “already” message and exits 0 without
  changing the row, including its timestamp.
- A missing UUID prints an error to stderr and exits nonzero. Invalid arguments,
  configuration failures, and database failures also exit nonzero. `--help` exits 0.
- Suspension remains unchanged, including for suspended accounts. Granting the
  role does not allow a suspended account to sign in.

Reload the app after a grant to refresh `/api/me`. Existing sessions use the current
database role. Grants persist across restarts and work with all linked identities.
Further promotions are available in `/admin`. Provider sign-in never grants roles
from email claims; `--initial-admin-email` is removed and rejected at startup.
Remove the old `command` entry from deployed Compose files before recreating the
API container with this version.

### Development forging

With a debug API build and `auth.dev_login = true`, **Forge a session** offers an
unchecked **Grant site administrator privileges** checkbox. It sends the boolean
`grant_admin` field, which defaults to false when omitted:

```sh
curl -i -X POST http://127.0.0.1:8080/auth/dev/login \
  -H 'content-type: application/json' \
  -d '{"subject":"local-admin","grant_admin":true}'
```

The shared sign-in transaction promotes new or returning accounts only when that
field is true, and commits the role together with session creation. False or omitted
values preserve existing roles. Suspended accounts still return 403; failed session
creation rolls back promotion. The subject remains an opaque provider identity key.
The handler module, route, development identity variant, grant input, and grant SQL
all use `#[cfg(debug_assertions)]` and are absent from release builds.

### Security assessment

The operator tool requires container execution access and database credentials;
protect both as administration access. Development forging can impersonate any
account and grant administration, so enable it only in trusted local environments.
Production uses the release Docker build with debug assertions explicitly disabled.
The publishing workflow waits for checks, including a release test that requests an
admin forged session with development login enabled and verifies 404 with no cookie.

Linked identities can sign in to a promoted account. Protect provider accounts and
linking/recovery methods. The API does not require fresh MFA or reauthentication
before destructive administration actions; modal confirmation prevents accidental
clicks.

## Account list and detail

Both account lists and a user's voyage list use descending UUID pagination, with
25 records by default and an optional `limit` from 1 through 100. `next_before` is
null on the last page; otherwise pass it as `before`. The UI provides Previous and
Next controls. Newly created UUIDv7 records appear ahead of an existing cursor.

Each account summary includes its ID, display name (nullable), creation time,
role, suspension status, photo count, voyage count, and `storage_bytes`. Counts
include drafts and photos in every processing state. Storage measurements sum
original and variant file lengths for distinct content hashes belonging to that
account. Duplicate uploads within an account count once; content shared between
accounts counts in each account's total. These are per-account file totals, not
exclusive disk usage. They exclude temporary uploads, database storage, filesystem
allocation overhead, and files awaiting cleanup without photo references. Missing
files count as zero; other filesystem errors fail the request. Conversion and
concurrent mutations can change totals between measurements. `AccountStorage` caches
at most 256 account totals per API process for 60 seconds after measurement; the list
and detail pages share that cache. Navigating voyage pages does not repeat a recent
filesystem scan. Expired entries are measured on the next request, and the oldest
measurement is evicted when the cache is full. Roles, suspension, and counts are read
from the database on every request and are never authorized from this cache.

A cache miss reads distinct hashes in pages of 1,000 and measures each page on the
blocking pool after releasing its query connection. The cache mutex protects only
cache lookups and updates, never database or filesystem work. Per-account locks
let concurrent callers share a measurement while cached reads and unrelated
accounts proceed. Overlapping pages acquire account locks in UUID order; cancellation
releases them, and later callers can retry. Errors do not cache partial totals. Storage changes can take up to a
cache lifetime to appear; a first measurement of a large account still scales with
its number of distinct hashes. Multiple API processes maintain independent caches.

The user detail screen lists the title, description, and publication status of
both drafts and published voyages. Administrators do not receive private photo
metadata, exact coordinates, or special access to image variants. Normal draft
voyage/photo endpoints still return 404 to an administrator who is not the owner.

## Confirmed actions

Every action opens a native modal identifying the account and explaining the
change. Cancel closes it without a request. Confirm submits the action; controls
are disabled while it is pending, and a failed request remains in the modal for
retry. Success refreshes affected queries. Deletion returns to the account list.

| Action | Result |
| --- | --- |
| Suspend account | Set `suspended`, revoke every session, and delete pending provider-link attempts. Reject future sign-ins, linking callbacks, and authenticated requests. Published voyages remain public. |
| Enable account | Clear `suspended`, discard any remaining sessions/link attempts, and permit a fresh sign-in. Previously revoked cookies stay invalid. |
| Promote to administrator | Set `is_site_admin`. Existing active sessions receive the new API permissions immediately; reload refreshes browser navigation. |
| Unpublish voyage | Set the selected owned voyage to draft and clear `published_at`. Another account's voyage returns 404. |
| Unpublish all voyages | Set every voyage owned by the selected account to draft and clear publication timestamps, including voyages outside the visible page. |
| Delete account and all data | Delete the account, identities, sessions, pending linking attempts, voyages, and photo records in one transaction. Reserve former display names and queue removal of original/variant files. Shared files survive while other photos reference them. |

Administrators cannot suspend or delete their own account (409). Administrator
mutations serialize on a transaction advisory lock and recheck the actor after
acquiring it. Two administrators trying to suspend or delete each other cannot
both succeed. Returning sign-ins hold an account write lock; linking holds a shared
account lock. Suspension therefore either revokes the newly created session or
completes first and causes sign-in rejection.
Uploads lock the account before the voyage, recheck suspension after receiving
bytes, and then lock photo content. Account deletion takes those locks in the same
order before gathering hashes and deleting rows. Other requests already authorized
when suspension begins may finish. Suspension does not remove public content;
unpublish separately when needed. An enabled owner can publish a moderated voyage
again; unpublishing is not a permanent publication ban.

Deletion schedules each content hash in `photo_cleanup` before commit and returns
204 immediately after that commit. Worker maintenance removes files in the background,
starting on its next pass (normally every 60 seconds; pending conversion recovery and
earlier cleanup can delay it). The HTTP handler performs no filesystem removal.
A failed removal is logged and remains queued for worker maintenance.
Each maintenance pass attempts at most 1,000 due records. A failed background attempt
sets `next_attempt_at` 60 seconds ahead so persistent failures do not block newer work.
The maintenance task awaits each pass before starting another; separate
API processes can attempt cleanup concurrently, serialized per content hash.
A successful 204 confirms database deletion and durable cleanup scheduling; it does
not mean the files have been removed.
Reserved display names remain without an account association. Unlinked sign-in
attempts have no known account and expire normally. Deletion does not ban a provider
identity from registering a new account later. Use suspension to prevent that
identity from signing in. Backups and external logs follow their own retention.

## API and implementation

| Method and path | Response |
| --- | --- |
| `GET /api/admin/users?before=UUID&limit=25` | `{ users: AdminUser[], next_before: UUID \| null }` |
| `GET /api/admin/users/{id}?before=UUID&limit=25` | `{ user: AdminUser, voyages: AdminVoyage[], next_before: UUID \| null }` |
| `POST /api/admin/users/{id}/actions` | 204 on success; missing accounts return 404. |

Action bodies use `{"action":"suspend"}`, `enable`, `promote`, `delete`, or
`unpublish_all`. Single-voyage unpublishing uses
`{"action":"unpublish_voyage","voyage_id":"UUID"}`. A missing voyage ID returns
400. Unknown actions/fields and malformed JSON use axum's JSON rejection responses.
The HTTP API enforces permissions; confirmation is a browser interaction, not an
additional authentication credential.

Implementation references:

- `crates/api/src/bin/pinvoyager-admin.rs::run` validates UUID arguments and
  performs the operator grant. `auth/dev.rs::login` and `auth/sign_in.rs::sign_in`
  implement the debug-only optional grant with session creation.
- `crates/api/src/http/admin.rs::{list, detail, act, measure_storage}` implements
  paging, role checks, and transactional moderation. `db/repo/users.rs::account_summaries`
  supplies the shared list/detail projection; `photos/account_storage.rs::AccountStorage`
  supplies bounded caching and paged storage measurements.
- `crates/api/src/db/repo/users.rs::require_active`,
  `crates/api/src/auth/extract.rs::resolve`, and
  `crates/api/src/auth/sign_in.rs::sign_in` enforce account status for
  provider callbacks and development forging. `http/auth.rs::{link, me}` enforce
  linking/account status and expose the signed-in role.
- `crates/api/src/http/photos.rs::ingest_part` serializes upload acceptance with
  account deletion/suspension; `photos/cleanup.rs::{schedule, clean, sweep}` removes
  unreferenced files and retries failures.
- `web/src/screens/Admin.tsx::{Admin, AdminContent, ConfirmAction}` supplies
  navigation, pagination, confirmation, pending states, errors, and query refresh.
  `web/src/api/{client,types}.ts` defines the matching transport and response types.
- `crates/api/tests/admin.rs` checks authorization, paging, counts, file totals,
  private-draft restrictions, moderation, cascades, shared files, retryable cleanup,
  concurrent administrator actions, and storage-cache reuse between list and detail.
  `photos/account_storage_tests.rs` checks expiry, eviction, measurement pagination,
  and retry after filesystem errors. `http/auth_tests.rs` checks suspended
  provider sign-in and linking. `web/src/screens/__tests__/Admin.test.tsx` checks
  list/detail behavior and confirmation for every action. The admin JSON fixtures
  are compared with normalized router responses and checked by
  `web/src/api/__tests__/contract.test.ts`.

Bootstrap tests: `crates/api/tests/admin_cli.rs` runs the actual operator binary
for invalid arguments, help, missing accounts, new grants, unchanged existing
administrators, suspended accounts, and database failure. `admin.rs::development_login_optionally_grants_admin`
checks optional grants, immediate authorization, concurrent sign-ins, retained roles,
and suspension rejection. `auth/dev_tests.rs::session_failure_rolls_back_new_accounts_and_existing_account_changes`
checks rollback. `auth_flow.rs::verified_email_does_not_grant_admin` checks provider
email claims cannot grant roles. `auth_build_mode.rs` verifies release route absence.
`web/src/auth/__tests__/DevSignIn.test.tsx` checks the checkbox default and request.
