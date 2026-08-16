# Phase 3 — Authentication

**Status:** not started
**Depends on:** phase 2
**Owner:** Claude

## Goal

A person can sign in with Google, Microsoft, Apple, or Facebook; a `Users` row and an
`Identities` row are created on first sign-in; a server-side session persists across restarts;
and a signed-in user can link an additional provider to the same account. No credential of any
kind is stored, per D1.

## Non-goals

- No frontend sign-in screen (phase 6). This phase is exercised over HTTP directly.
- No account deletion (phase 8).
- No roles, permissions, or admin concepts.
- No email sending of any kind.
- **No ASP.NET Core Identity** (see phase 2). External authentication plus cookie
  authentication, nothing more.

## Targets

1. Provider registration using the framework's external authentication handlers:
   `Microsoft.AspNetCore.Authentication.Google`, `.MicrosoftAccount`, `.Facebook` (all
   first-party), and `AspNet.Security.OAuth.Apple` from the aspnet-contrib project for Apple.
   These handle the authorization-code dance, `state`, `nonce`, PKCE, and JWKS retrieval —
   the work in this phase is the identity mapping around them, not the protocol itself.
2. Cookie authentication as the application scheme, with the external schemes used only to
   establish who the caller is and then signed out immediately.
3. **Server-side, revocable sessions** via a custom `ITicketStore`, backed by a `Sessions`
   table. The cookie carries an opaque key; the ticket lives in PostgreSQL. Cookie configured
   `HttpOnly`, `Secure`, `SameSite=Lax`.
4. **Persisted Data Protection key ring** — stored in PostgreSQL (or a mounted volume) rather
   than left in its default per-container location. Without this, session cookies are
   unreadable after a restart and unshareable across instances, which silently defeats T3.
5. Identity mapping in `OnTicketReceived`: look up `(provider, subject)`; create `Users` +
   `Identities` on first sign-in; load the existing user on a return visit; then issue the
   application cookie for the local user.
6. Apple specifics: `response_mode=form_post`, a client secret that is a **JWT you generate and
   sign yourself** with a key from Apple and which expires (six months maximum), and the user's
   name arriving **only on first authorization** in the form body. Capture it then or lose it
   permanently.
7. **Apple correlation-cookie fix:** because Apple POSTs the callback cross-site, a
   `SameSite=Lax` correlation cookie is not sent and the handler fails with a correlation
   error. Apple's options need `CorrelationCookie.SameSite = SameSiteMode.None` with `Secure`.
   This is the single most common reason Sign in with Apple "mysteriously" fails in ASP.NET
   Core.
8. Facebook is OAuth2, not OIDC — no ID token exists. The handler already fetches the profile
   from the Graph API; the app-scoped ID is the subject.
9. `POST /auth/link/{provider}` — links an additional provider to the **already signed-in**
   user. If that identity is attached to a different account, it fails with a clear conflict
   and changes nothing.
10. `POST /auth/logout` — deletes the session row through the ticket store, not just the cookie.
11. `GET /api/me` — returns the current user and their linked providers.
12. `RequireAuthorization()` on protected endpoints, with a typed accessor for the current
    user id, so authorization is structural rather than a forgotten `if`.

## Acceptance criteria

- **T1/T5** — Full round trip against each provider's real sandbox produces a session cookie
  and a `Users` row. Tampering with `state` is rejected. A replayed authorization code is
  rejected.
- **T3** — Deleting the session row signs the user out immediately on the next request. A
  self-contained encrypted ticket could not do this, which is why the ticket store exists.
- **T4** — **Restart the process: the user stays signed in.** If they are signed out, the key
  ring is not persisted. Then run two instances behind a proxy and confirm a cookie issued by
  one is accepted by the other.
- **T5** — Signing in twice with the same provider produces exactly one `Users` row.
- **T6/T7** — Apple's POST callback succeeds end to end. Revoking and re-authorizing
  demonstrates the name is absent the second time, and the code handles it. Confirm the
  generated client secret's expiry is tracked somewhere a human will see before it lapses.
- **T9** — Linking a second provider yields one user with two identities, and both sign in to
  it. Attempting to link an identity already owned by another account returns a conflict and
  leaves both accounts untouched.
- **Security** — No table in the schema holds a password, hash, or long-lived provider token.
  Verify by inspecting the schema, not by intent. Confirm the external handlers are configured
  with `SaveTokens = false`, which is the default but is worth asserting given D1.

## Design references

- `design/screens/sign-in.html`
- `design/screens/auth-states.html` — callback, conflict, and expiry states
- `design/flows/auth.html`
- `design/components/provider-buttons.html`

## Open questions

- **Session lifetime and idle timeout.** Not yet chosen. Sliding expiry is friendlier;
  absolute expiry is safer. The ticket store makes either enforceable server-side.
- **Email storage.** Do we store the provider-supplied email at all? It is not needed for
  sign-in (D3 matches on subject) and not needed for notification (no email is sent). Storing
  it creates PII we otherwise would not hold. Recommendation: do not store it, and strip the
  email claim before the principal is persisted.
- **Apple private relay.** Apple lets users hide their real address behind a relay. Moot if we
  do not store email; worth confirming alongside the question above.
- Whether to keep the handlers' default callback paths or route them under `/auth/`. The
  defaults must match what is registered in each provider's console either way.

## Notes

**The protocol work is mostly done for you here, and that changes this phase's character.**
In a hand-rolled implementation the risk is getting verification wrong. With the framework
handlers, signature, issuer, audience, expiry, and nonce validation are handled — so the risk
shifts to *configuration*: a wrong `Authority`, `SaveTokens` left on, a correlation cookie
that never arrives, or an unpersisted key ring. The flow diagram in
`design/flows/auth.html` still shows every gate, because knowing what the handler is doing on
your behalf is what lets you tell a real failure from a misconfiguration.

**Never match accounts on email.** Two providers can return the same address for the same
human, but not every provider verifies address ownership, so auto-merging on email is an
account-takeover vector. Linking is always an explicit action by an authenticated user (D3).

**Build Apple first, not last.** Between the form-POST callback, the self-signed expiring
client secret, the one-shot name delivery, and the correlation-cookie `SameSite` trap, it is
by a wide margin the most awkward of the four. Discovering that at the end distorts the whole
phase.
