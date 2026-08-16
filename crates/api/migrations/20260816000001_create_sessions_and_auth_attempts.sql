-- Phase 3: server-side sessions and in-flight authorization attempts.
--
-- D1 still holds. Neither table stores a user credential: no password, no
-- hash of one, and no provider access or refresh token. The provider's ID
-- token is verified on receipt and discarded — nothing here persists it.

-- Sessions are server-side so they can be revoked. A self-contained signed
-- cookie cannot be invalidated before it expires, which matters more here than
-- usual: with social-only sign-in the account *is* the identity, and there is
-- no password to change if something goes wrong.
CREATE TABLE sessions (
    -- SHA-256 of the session token, never the token itself. The raw token
    -- exists only in the user's cookie.
    --
    -- This is not paranoia about D1 — it is the same reasoning applied to a
    -- credential we issue rather than one we are given. A database leak that
    -- exposed raw session tokens would hand over live authenticated sessions
    -- for every signed-in user, and hashing costs one SHA-256 per request to
    -- prevent that. Lookup is by hash, which is deterministic, so this costs
    -- nothing in index terms.
    token_hash    bytea       PRIMARY KEY,

    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    created_at    timestamptz NOT NULL DEFAULT now(),

    -- Sliding expiry: each authenticated request pushes this forward.
    last_seen_at  timestamptz NOT NULL DEFAULT now(),

    -- Absolute ceiling, fixed at creation and never extended. A sliding
    -- timeout on its own means a stolen cookie stays valid indefinitely so
    -- long as it keeps being used; this is what eventually forces a fresh
    -- sign-in regardless of activity.
    absolute_expires_at timestamptz NOT NULL
);

-- "Sign me out everywhere" and the account-deletion cascade both need this.
CREATE INDEX sessions_user_id_idx ON sessions (user_id);

-- Sweeping expired sessions is a range scan over this.
CREATE INDEX sessions_absolute_expires_at_idx ON sessions (absolute_expires_at);

-- An authorization attempt in flight: created at /auth/{provider}/start,
-- consumed at the callback, and single-use.
--
-- This table is what makes the state and nonce checks real. Holding them in a
-- cookie instead would work, but it puts the anti-CSRF value in the same place
-- as the thing it protects; keeping it server-side means a callback can only
-- succeed if this service actually started that exact flow.
CREATE TABLE auth_attempts (
    -- The OAuth2 `state` parameter. Stored as-is rather than hashed: unlike a
    -- session token it is single-use, expires in minutes, and is already
    -- visible in the redirect URL, browser history, and the provider's logs.
    -- Hashing it would add ceremony without removing an exposure.
    state           text        PRIMARY KEY,

    provider        text        NOT NULL,

    -- Replay protection for the ID token. Must match the `nonce` claim.
    nonce           text        NOT NULL,

    -- PKCE. Sent at the code exchange to prove this service began the flow.
    pkce_verifier   text        NOT NULL,

    -- Set when the flow is a link-an-additional-provider attempt rather than a
    -- sign-in. D3 requires linking to be an action taken by an already
    -- authenticated user, so the callback must know which user began it and
    -- must not infer it from the returned identity.
    link_user_id    uuid        REFERENCES users (id) ON DELETE CASCADE,

    -- Where to send the browser afterwards. Validated against an allowlist
    -- before use — an unchecked value here is an open-redirect.
    redirect_to     text,

    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL,

    CONSTRAINT auth_attempts_provider_check
        CHECK (provider IN ('google', 'microsoft', 'apple', 'facebook'))
);

-- Expired attempts are swept on this.
CREATE INDEX auth_attempts_expires_at_idx ON auth_attempts (expires_at);
