# Authentication flow

No credentials of any kind are stored — no passwords, no hashes, no long-lived
provider tokens. The provider's ID token is verified on receipt and discarded.
What persists is the fact of the identity: which provider, and that provider's
stable subject.

Two tables carry it: `users` (the account, owns voyages) and `identities` (one
row per linked provider, unique on `(provider, subject)`).

## Sign-in, end to end

```mermaid
sequenceDiagram
    autonumber
    actor Browser
    participant SignIn as SignIn screen
    participant Start as GET /auth/{provider}/start
    participant DB as Postgres
    participant Provider as Google / Microsoft / Apple / Facebook
    participant CB as GET /auth/{provider}/callback
    participant Me as GET /api/me

    Browser->>SignIn: /signin?redirect_to=/upload
    SignIn->>SignIn: destinationOr(redirect_to) — allowlist check
    SignIn->>Start: window.location.assign(...)

    Start->>Start: parse_provider, provider.authorize()
    Note over Start: mints state, nonce, PKCE verifier,<br/>and a binding value
    Start->>DB: INSERT auth_attempts (state, nonce,<br/>pkce_verifier, binding_hash,<br/>redirect_to validated first, expires_at)
    Start-->>Browser: 302 to the provider + Set-Cookie: binding

    Browser->>Provider: authorization request (PKCE challenge, state, nonce)
    Provider-->>Browser: consent
    Browser->>CB: redirect back with code and state<br/>(Apple: POST form_post)

    CB->>DB: DELETE auth_attempts WHERE state = ? RETURNING *
    Note over CB,DB: atomic consume — a replay fails on<br/>its second use whatever else it carries

    alt no attempt matched
        CB-->>Browser: 400, one generic sentence
    else attempt consumed
        CB->>CB: binding cookie matches binding_hash?
        CB->>CB: attempt.provider == path provider?
        CB->>CB: callback carried an error?
        CB->>Provider: exchange(code, pkce_verifier, nonce)
        Note over Provider: JWKS signature, iss, aud, exp, nonce.<br/>Facebook instead: OAuth2 exchange + Graph lookup
        Provider-->>CB: VerifiedIdentity { provider, subject }
        CB->>CB: identity.provider == path provider?

        CB->>DB: BEGIN
        CB->>DB: SELECT identities WHERE (provider, subject)
        alt identity exists
            DB-->>CB: user_id
        else first sign-in
            CB->>DB: INSERT users, INSERT identities
        end
        CB->>DB: INSERT sessions (token hash)
        CB->>DB: COMMIT
        CB-->>Browser: 303 to redirect_to + Set-Cookie: session (HttpOnly)
    end

    Browser->>Me: GET /api/me with the session cookie
    Me-->>Browser: { user, linked providers }
```

### The four checks before the code is exchanged

Order matters. The attempt is consumed before anything else is looked at, so no
branch below can be reached without a `state` this service issued
(`crates/api/src/http/auth.rs:401-448`).

```mermaid
graph TD
    A["callback arrives"] --> B{"state present?"}
    B -->|no| X["400 — generic"]
    B -->|yes| C["DELETE ... RETURNING — atomic consume"]
    C -->|"no row: unknown, used, or expired"| X
    C -->|"row"| D{"binding cookie matches<br/>attempt.binding_hash?"}
    D -->|no or absent| X
    D -->|yes| E{"attempt.provider ==<br/>path provider?"}
    E -->|no| X
    E -->|yes| F{"provider returned<br/>an error?"}
    F -->|"access_denied"| Y["400 — canceled"]
    F -->|"other"| X
    F -->|no| G["exchange + verify with the provider"]
    G --> H{"identity.provider ==<br/>path provider?"}
    H -->|no| X
    H -->|yes| I{"attempt.link_user_id set?"}
    I -->|no| J["sign_in: find or create user,<br/>issue session cookie"]
    I -->|yes| K["link: attach identity to that user,<br/>no new session"]
```

The binding cookie is what closes link CSRF. An attacker can hand a victim a URL
carrying the attacker's `state`, but cannot put the matching cookie in the
victim's browser. Only the hash is stored, so a database reader cannot complete
someone's in-flight authorization.

`sign_in` looks up `(provider, subject)` and nothing else. There is no email
branch and no email column to write one with. Two providers can return the same
address for the same human while not every provider verifies ownership, so
matching on email is an account-takeover vector.

## Linking a second provider

Started from `/settings`, which posts a form to `/auth/link/{provider}`.

```mermaid
sequenceDiagram
    autonumber
    actor Browser
    participant Settings as Settings screen
    participant LinkStart as POST /auth/link/{provider}
    participant DB as Postgres
    participant Provider
    participant CB as callback

    Browser->>Settings: signed in as user A
    Settings->>LinkStart: form POST, redirect_to=/settings
    Note over LinkStart: CurrentUser is an extractor, so an<br/>anonymous caller has no path through.<br/>POST because SameSite=Lax withholds the<br/>cookie from a cross-site POST.
    LinkStart->>DB: INSERT auth_attempts with link_user_id = A
    LinkStart-->>Browser: 303 to the provider + binding cookie
    Browser->>Provider: consent
    Provider-->>CB: code + state
    CB->>CB: the same four checks
    CB->>DB: INSERT identities (provider, subject, user_id = A)
    CB-->>Browser: 303 to /settings — no new session cookie
```

The account is the one that *started* the flow, read back from
`attempt.link_user_id`. Using the account implied by the returned identity would
let anyone who can complete a provider round trip attach it to whichever account
they liked. No session is issued: the caller is already signed in as A, and that
session stays exactly as it was.

## Session state in the browser

```mermaid
stateDiagram-v2
    [*] --> loading: AuthProvider mounts, useMe() fires
    loading --> authenticated: 200 with a user
    loading --> anonymous: 401, swallowed by useMe
    authenticated --> anonymous: POST /auth/logout
    anonymous --> loading: refresh() after a sign-in round trip
    authenticated --> loading: refresh()
```

The 401 is swallowed on purpose (`web/src/api/hooks.ts:135`). Most of this
application is readable without an account, so "not signed in" is an ordinary
state of the shell. A `useMe` that surfaced a 401 as `isError` would put an
error boundary in front of the discovery feed for every signed-out visitor.

`AuthProvider` sits outside the router (`web/src/App.tsx:64`) so one read of
`/api/me` serves the whole session rather than one per route.
