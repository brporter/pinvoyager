# Phase 3 — Authentication

**Status:** in progress — blocked on provider credentials
**Depends on:** phase 2
**Owner:** Claude

Every target is implemented and the build is green: `cargo fmt --check`, `cargo clippy
--all-targets -- -D warnings`, the `SQLX_OFFLINE=true` build, and 66 tests (49 lib, 9
crafted-token, 7 integration, 1 doctest).

**Verified without credentials:** the whole of T3's crafted-token half, against a local mock
OIDC provider — wrong signing key, unknown `kid` (with a bounded JWKS refetch), wrong `aud`,
wrong `iss`, expired, mismatched `nonce`, and a token carrying *no* `nonce` at all. Plus
Apple's ES256 client-secret JWT signature and claims, session sliding expiry and its absolute
ceiling, atomic single-use consumption of auth attempts under genuine concurrency, and the
open-redirect allowlist.

**Cannot be verified here:** T2's "full round trip against each provider's real sandbox".
No provider credentials exist on this machine. Everything touching a real provider — discovery,
the code exchange, verification against a real JWKS, the rotation retry firing, Apple's
`form_post` round trip and the shape of its one-shot `user` payload, and whether Apple accepts
the client secret we sign — is unexercised.

## Goal

A person can sign in with Google, Microsoft, Apple, or Facebook; a `users` row and an
`identities` row are created on first sign-in; a server-side session persists across restarts;
and a signed-in user can link an additional provider to the same account. No credential of any
kind is stored, per D1.

- **Session lifetime and idle timeout.** Sliding timeout.
- **Email storage.** Don't store email address.
- **Apple private relay.** Since we don't store email addresses, we don't care about fake addresses
  created using Apple private relay.

## Non-goals

- No frontend sign-in screen (phase 6). This phase is exercised over HTTP directly.
- No account deletion (phase 8).
- No roles, permissions, or admin concepts.
- No email sending of any kind.

## Targets

1. A provider abstraction — a trait with per-provider implementations — so that the three
   OIDC providers share code and Facebook's OAuth2-plus-Graph path fits without special-casing
   the call sites.
2. `GET /auth/{provider}/start` — generates `state` and `nonce`, derives a PKCE challenge,
   stores the pending attempt server-side with a short TTL, and redirects to the provider.
3. `GET /auth/{provider}/callback` — validates `state`, exchanges the code with PKCE, and
   verifies the ID token: signature against cached JWKS, plus `iss`, `aud`, `exp`, and `nonce`.
4. `POST /auth/apple/callback` — Apple uses `response_mode=form_post`, so its callback is a
   POST with a form body, not a GET with query parameters. It also returns the user's name
   **only on first authorization**; capture it then or lose it permanently.
5. Facebook path — code exchange followed by a Graph API `/me` call, using the app-scoped ID
   as the subject. There is no ID token to verify.
6. JWKS fetching with caching and honest key rotation handling: cache by `kid`, refetch on
   unknown `kid`, and rate-limit refetches so an attacker cannot use bogus `kid`s to drive
   traffic at the provider.
7. First sign-in creates `users` + `identities`. Returning sign-in matches on
   `(provider, subject)` and updates nothing but last-seen.
8. Server-side sessions: opaque high-entropy ID in a cookie set `HttpOnly`, `Secure`,
   `SameSite=Lax`, with the session record in PostgreSQL. Revocable.
9. `POST /auth/link/{provider}` — links an additional provider to the **already signed-in**
   user. If that provider identity is already attached to a different account, it fails with a
   clear conflict error and changes nothing.
10. `POST /auth/logout` — destroys the session server-side, not just the cookie.
11. `GET /api/me` — returns the current user and their linked providers.
12. An extractor that makes a handler require authentication, so authorization is a type-level
    property rather than a forgotten `if`.

## Acceptance criteria

- **T2/T3** — Full round trip against each provider's real sandbox produces a session cookie
  and a `users` row. Tampering with `state` is rejected. A replayed code is rejected.
- **T3** — A token signed by the wrong key, with the wrong `aud`, expired, or with a
  mismatched `nonce` is rejected in each case. Test with crafted tokens, not only live ones.
- **T4** — Apple's POST callback succeeds and the name is persisted from the first
  authorization. Revoking and re-authorizing demonstrates the name is absent the second time —
  and the code handles it.
- **T7** — Signing in twice with the same provider produces exactly one `users` row.
- **T8** — Restarting the process keeps the user signed in. Deleting the session row signs
  them out immediately.
- **T9** — Linking a second provider yields one user with two identities and both can sign in
  to it. Attempting to link an identity already owned by another account returns a conflict
  and leaves both accounts untouched.
- **Security** — No table in the schema holds a password, hash, or long-lived provider token.
  Verify by inspecting the schema, not by intent.

## Design references

- `design/screens/sign-in.html`
- `design/screens/auth-states.html` — callback, conflict, and expiry states
- `design/flows/auth.html`
- `design/components/provider-buttons.html`

## Open questions

- ~~**Link CSRF at the provider**~~ — **RESOLVED.** Every authorization attempt is now bound
  to the browser that started it: `/auth/{provider}/start` mints a 256-bit value, sets it as a
  `SameSite=None; Secure; HttpOnly` cookie, and stores its SHA-256 on the attempt row; the
  callback must present a cookie hashing to that value or the flow is refused. `SameSite=None`
  is what lets it survive Apple's cross-site form POST — the reason the obvious
  session-matching fix could not be used. Checked on sign-in as well as linking, since one
  code path is easier to keep correct than two. Original analysis retained below.

  T9 requires linking to be an authenticated action, and it is: the attempt records
  `link_user_id` from a request carrying a valid session. That closes "prove B, claim A". It
  does **not**, on its own, close the reverse:

  1. The attacker signs in to their own account and starts a link flow, getting a provider URL
     whose `state` is bound to an attempt with `link_user_id = attacker`.
  2. They do not follow it. They lure a victim into clicking it.
  3. The victim is signed in at the provider, sees a normal consent screen, and approves.
  4. The provider redirects the *victim's* browser to our callback with the attacker's `state`.
  5. We consume the attempt, read `link_user_id = attacker`, and attach the **victim's**
     provider identity to the **attacker's** account.

  The attacker gains nothing from the victim's existing data — if the victim already has an
  account under that identity, the unique index refuses the link. The damage is to a victim who
  has *not* signed up yet: their future sign-ins with that provider land in the attacker's
  account. This is account pre-hijacking, and social-only sign-in (D2) is exactly the shape
  where it bites, because there is no password step to interrupt it.

  **The obvious mitigation does not work.** Requiring the callback's session to match
  `attempt.link_user_id` fails for Apple: its callback is a cross-site form POST, and a
  `SameSite=Lax` cookie is not sent on one, so every Apple link would break.

  **The fix that does work** is binding the attempt to the browser that started it — a
  separate high-entropy value set as a `SameSite=None; Secure` cookie at start, its hash stored
  on the attempt, required to match at the callback. That is a deliberate decision: `SameSite=None`
  cookies carry their own considerations, and it adds a cookie to the flow.

  Decide before phase 6 puts a link button in front of anyone.


## Notes

**Never match accounts on email.** Two providers can return the same address for the same
human, but not every provider verifies address ownership, so auto-merging on email is an
account-takeover vector. Linking is always an explicit action taken by an authenticated user
(D3).

**Apple is the awkward one** and should be built first, not last: form-POST callback, a client
secret that is a JWT you generate and sign with a key from Apple and which expires (six months
maximum), and one-shot name delivery. Discovering this late distorts the whole phase.

**Facebook is not OIDC.** Do not attempt ID token verification against it.

### Two dependency traps, found the hard way

Both were introduced by taking `cargo add` defaults, and both are the kind that pass a build.

**`jsonwebtoken` ships with no crypto backend enabled.** Its default features are `use_pem`
alone; neither `rust_crypto` nor `aws_lc_rs` comes in by default, and without one the crate can
neither sign nor verify — `CryptoProvider::get_default()` panics at *runtime*. Confirmed by
inspection: nothing under it in the dependency tree provides RSA or P-256.

This is nastier than a compile error. Apple's client secret is the only thing in this service
that signs a JWT, so the panic would first appear the moment someone tried to sign in with
Apple — in whatever environment that happened to be. Enable `rust_crypto` (pure Rust, no
native toolchain, consistent with keeping the build simple) or `aws_lc_rs`.

**Two incompatible `reqwest` versions.** `openidconnect` 4 depends on `oauth2` 5, which is
built against `reqwest` 0.12; adding `reqwest` directly pulls 0.13, and both end up in the
lock file. A `&reqwest::Client` from 0.13 does not implement `oauth2`'s `AsyncHttpClient`, so
`CoreProviderMetadata::discover_async` will not typecheck against it. The direct dependency
must be pinned to 0.12 so there is one client type, not two that look identical.

The general lesson: `cargo add <crate>` takes default features, and for security-relevant
crates the default is often "no backend chosen". Check the feature list rather than assuming
the obvious capability is on.
