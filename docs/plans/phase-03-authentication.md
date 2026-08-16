# Phase 3 — Authentication

**Status:** not started
**Depends on:** phase 2
**Owner:** Claude

## Goal

A person can sign in with Google, Microsoft, Apple, or Facebook; a `users` row and an
`identities` row are created on first sign-in; a server-side session persists across restarts;
and a signed-in user can link an additional provider to the same account. No credential of any
kind is stored, per D1.

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

- **Session lifetime and idle timeout.** Not yet chosen. Sliding expiry is friendlier;
  absolute expiry is safer.
- **Email storage.** Do we store the provider-supplied email at all? It is not needed for
  sign-in (D3 matches on subject) and not needed for notification (no email is sent). Storing
  it creates PII we otherwise would not hold. Recommendation: do not store it.
- **Apple private relay.** Apple lets users hide their real address behind a relay. Harmless
  if we do not store email; worth confirming alongside the question above.

## Notes

**Never match accounts on email.** Two providers can return the same address for the same
human, but not every provider verifies address ownership, so auto-merging on email is an
account-takeover vector. Linking is always an explicit action taken by an authenticated user
(D3).

**Apple is the awkward one** and should be built first, not last: form-POST callback, a client
secret that is a JWT you generate and sign with a key from Apple and which expires (six months
maximum), and one-shot name delivery. Discovering this late distorts the whole phase.

**Facebook is not OIDC.** Do not attempt ID token verification against it.
