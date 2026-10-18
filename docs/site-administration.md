# Site administration

Site administrators use `/admin` to list accounts and `/admin/users/:id` to manage
one account. The account menu includes **Site administration** only when
`GET /api/me` reports `user.is_site_admin: true`. The browser checks that role
before fetching administration data; every administration API request independently
checks the current database role. Anonymous requests return 401 and signed-in
non-administrators return 403.

The migration `20261018000001_site_administration.sql` adds `users.is_site_admin`
and `users.suspended`, both defaulting to false. Public profile updates cannot grant administration access. Provider and development sign-in can
grant the role through the startup allowlist described below. Public profile responses do not expose
the administration flag.

## Bootstrap

Start the API with one `--initial-admin-email` argument per address:

```sh
cargo run -p pinvoyager-api -- \
  --initial-admin-email first@gmail.com \
  --initial-admin-email second@example.com

# Installed binary; the equals form is also accepted.
pinvoyager-api --initial-admin-email=first@gmail.com --initial-admin-email=second@example.com
```

`--help` describes the arguments. Unknown arguments, missing values, non-ASCII
addresses, missing local/domain parts, multiple `@` signs, and embedded
whitespace/control characters fail startup. Comparisons trim surrounding
whitespace, preserve local-part case, and ignore ASCII case in the domain. They
preserve dots and plus suffixes; no Gmail
alias expansion or domain-wide matching occurs. Duplicate arguments count once.
The allowlist is supplied only through these command-line arguments, not TOML,
environment variables, HTTP input, or provider display names.

On each successful provider sign-in, after token verification, the API promotes
the account if it is not already an administrator and its eligible token email
matches an entry. Promotion and session creation commit together. Suspended
accounts cannot sign in or gain a role. Existing sessions do not trigger this
check; the user must sign in again. The first signed-in `/api/me` response exposes
the role, and `/admin` is immediately authorized. Linking a provider does not trigger promotion.

Development-only **Forge a session** also applies the allowlist: its supplied
`subject` acts as the email input, for any selected provider, without token or email
verification. New and returning accounts receive the role through the shared
`auth/sign_in.rs::sign_in` transaction that also creates their session. Suspended accounts still return 403. Account identity continues
to match the exact provider/subject pair, so entering a different subject creates
a different identity even if the email comparison ignores domain case. Additional
`email` or `email_verified` request fields have no effect. This exception requires
a debug API build and `auth.dev_login = true`; the release build has no development
login route. It is a local testing bypass, not evidence of email ownership.

For real provider sign-in, email eligibility depends on the provider:

| Provider | Eligible token claims |
| --- | --- |
| Google | `email_verified` must be boolean true, and the address must end in `@gmail.com` or the token must have a nonempty `hd` claim for Google Workspace. |
| Apple | `email_verified` must be true or the string `"true"`. A private relay address matches only that relay address, not the underlying mailbox. |
| Microsoft | No email-based bootstrap. Microsoft's email claim is unsuitable for authorization. Use an existing administrator or an explicit operator database grant. |

Google and Apple request the `email` scope only while the allowlist is nonempty.
Claims come from the ID token after signature, issuer, audience, expiry, and nonce
verification. Emails remain transient and are never stored on user accounts,
returned by the account API, or included in the promotion log. Identity matching
continues to use provider and subject, never email.

Remove the arguments and restart all API instances after the intended accounts
have signed in. Grants persist in the database. Leaving an address configured
allows later matching sign-ins to grant administration again, including after an
operator removes the role. Removing an argument does not revoke an existing role.
Further promotions are available in the administration UI and require confirmation.
Promotion does not re-enable suspended accounts. Role removal currently requires
an operator database update; the UI supports promotion only.

The production `api` service in `deploy/vm/docker-compose.yml` supplies
`--initial-admin-email bryan@bryanporter.com` at startup. Apply the updated Compose
file on the server and recreate the API container for it to take effect. After an
eligible sign-in grants the role, remove that service's `command` entry and recreate
the container again to stop automatic grants.

An operator can alternatively grant an existing enabled account explicitly:

```sql
UPDATE users SET is_site_admin = true, updated_at = now()
WHERE id = '00000000-0000-0000-0000-000000000000' AND NOT suspended
RETURNING id, display_name, is_site_admin;
```

Replace the UUID with the intended account ID. Reload the app to refresh the
signed-in role. No database accounts are promoted automatically during migration.

### Security assessment

Email bootstrap is convenient but weaker than an explicit grant to a stable
provider identity. A stolen mailbox/provider account, recycled address, changed
domain ownership, or a trusted provider's compromised account-recovery process
can let someone satisfy the allowlist. Requiring verification is necessary but
not sufficient: Google warns that verification of an external address can become
stale, and Microsoft explicitly prohibits authorizing on its mutable email claim.
The provider restrictions above follow
[Google's token-verification guidance](https://developers.google.com/identity/gsi/web/guides/verify-google-id-token)
and [Microsoft's claim reference](https://learn.microsoft.com/en-us/entra/identity-platform/id-token-claims-reference).
Apple documents its verified and private-relay email claims in
[the authorization response reference](https://developer.apple.com/documentation/signinwithapplerestapi/request-an-authorization-to-the-sign-in-with-apple-server).

The allowlist remains an ongoing grant rule while configured; it is not consumed
once and is not disabled when the first administrator exists. Multiple provider
identities with the same eligible email can receive separate administrator
accounts. Existing linked identities also provide sign-in access to the promoted
account. Require strong provider account protection, preferably phishing-resistant
MFA, and control provider-linking/recovery methods for these accounts. The API does
not require fresh MFA or reauthentication before a destructive admin action; modal
confirmation only prevents accidental clicks.

Treat command-line arguments, provider issuer configuration, and deployment
manifests as privileged configuration. Email addresses may be visible in process
listings or shell history. The API's configuration debug output redacts the
allowlist, startup errors do not echo addresses, and successful grants log account
ID and provider after commit. Logs are not a durable administration audit ledger.
For ongoing policy, a provider issuer/subject allowlist or an explicit account-ID
grant avoids relying on a mutable email claim. Disable bootstrap after use and
review existing roles separately when changing the allowlist.

## Account list and detail

Both account lists and a user's voyage list use descending UUID pagination, with
25 records by default and an optional `limit` from 1 through 100. `next_before` is
null on the last page; otherwise pass it as `before`. The UI provides Previous and
Next controls. Newly created UUIDv7 records appear ahead of an existing cursor.

Each account summary includes its ID, display name (nullable), creation time,
role, suspension status, photo count, voyage count, and `storage_bytes`. Counts
include drafts and photos in every processing state. Storage is the sum of current
original and variant file lengths for distinct content hashes belonging to that
account. Duplicate uploads within an account count once; content shared between
accounts counts in each account's total. These are per-account file totals, not
exclusive disk usage. They exclude temporary uploads, database storage, filesystem
allocation overhead, and files awaiting cleanup without photo references. Missing
files count as zero; other filesystem errors fail the request. Conversion and
concurrent mutations can change totals between reads. Filesystem reads run on the
blocking pool, after releasing the database query connection.

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
both succeed. Sign-in and linking hold a shared account lock, so suspension either
revokes their newly created session or completes first and causes rejection.
Uploads lock the account before the voyage, recheck suspension after receiving
bytes, and then lock photo content. Account deletion takes those locks in the same
order before gathering hashes and deleting rows. Other requests already authorized
when suspension begins may finish. Suspension does not remove public content;
unpublish separately when needed. An enabled owner can publish a moderated voyage
again; unpublishing is not a permanent publication ban.

Deletion schedules each content hash in `photo_cleanup` before commit, then attempts
file removal. A failed removal is logged and remains queued for worker maintenance.
A successful 204 confirms database deletion; file removal may still be retrying.
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

- `crates/api/src/initial_admin.rs::InitialAdmins::parse_args` validates startup
  arguments; `main.rs::{entrypoint, run}` installs the allowlist before provider
  discovery. `auth/providers/oidc.rs::admin_email_from_verified_token` applies the
  provider claim policy; `auth/sign_in.rs::sign_in` commits any grant with the session.
- `crates/api/src/http/admin.rs::{list, detail, act, measure_storage}` implements
  paging, role checks, storage measurement, and transactional moderation.
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
  and concurrent administrator actions. `http/auth_tests.rs` checks suspended
  provider sign-in and linking. `web/src/screens/__tests__/Admin.test.tsx` checks
  list/detail behavior and confirmation for every action. The admin JSON fixtures
  are compared with normalized router responses and checked by
  `web/src/api/__tests__/contract.test.ts`.

Bootstrap tests: `initial_admin_cli.rs` runs the actual binary with help, invalid,
and repeated arguments. `auth_tokens.rs` and provider tests verify signed email
claims, forged-token rejection, provider differences, and conditional email scopes.
`auth_flow.rs::verified_bootstrap_email_grants_admin_before_the_callback_session_is_used`
checks the full callback/session/admin request. `http/auth_tests.rs` checks returning
accounts, persisted grants, concurrent sign-ins, no merging, and suspension/linking
restrictions. `admin.rs::development_login_applies_initial_admin_emails` verifies grants to new
and returning forged sessions, immediate administration access, concurrent
promotions, nonmatching subjects, removed configuration, and suspension rejection.
