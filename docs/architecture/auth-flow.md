# Authentication flow

[`http/auth.rs::routes`](../../crates/api/src/http/auth.rs) registers provider start,
GET/POST callback, provider-link start, logout, and current-user endpoints.
Google and Microsoft use OpenID Connect; Apple uses an OIDC form callback and
signed client secret; Facebook uses OAuth code exchange and provider identity lookup.

```mermaid
sequenceDiagram
    participant Browser
    participant API
    participant Provider
    participant DB as PostgreSQL
    Browser->>API: Start sign-in
    API->>DB: Store state, nonce, PKCE, and browser-binding hash
    API-->>Browser: Binding cookie and provider redirect
    Browser->>Provider: Authorize
    Provider-->>Browser: Callback with code and state
    Browser->>API: Callback plus binding cookie
    API->>DB: Consume authorization attempt
    API->>Provider: Exchange code and verify identity
    API->>DB: Resolve provider subject and create session hash
    API-->>Browser: Session cookie and frontend callback redirect
    Browser->>API: GET /api/me
    API-->>Browser: Account and linked providers
```

The browser-binding cookie is HttpOnly, Secure, and SameSite=None so the provider
form callback can carry it. Session cookies are HttpOnly and SameSite=Lax, with
Secure controlled by configuration. Identity matching never merges by email.
Provider linking starts with an authenticated account and refuses an identity
already linked to another account.

[`session::lookup_and_touch`](../../crates/api/src/auth/session.rs) enforces idle and
absolute expiration. Defaults are 14 days idle and 90 days absolute. Database activity
extends idle expiry. The browser cookie's Max-Age uses the absolute session limit; the
server still rejects sessions whose idle deadline has passed. Periodic maintenance
removes expired sessions and authorization attempts. Logout revokes the presented
session and clears the cookie; it accepts missing or expired sessions.

Provider callback failures redirect to the frontend result screen. Client-side
[`AuthCallback`](../../web/src/auth/AuthCallback.tsx) presents the status and a
validated local destination. [`AuthProvider`](../../web/src/auth/AuthProvider.tsx)
loads current-user data and requires a unique name when it is absent. See
[display names](../display-names.md).

Debug builds can expose `/auth/dev/login` when enabled in configuration. The route
is absent from release builds. See [local sign-in](../running.md#signing-in-without-provider-credentials).
