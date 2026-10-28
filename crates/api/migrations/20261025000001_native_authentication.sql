-- Native credentials share hashed, revocable sessions without adopting browser credentials.
CREATE TABLE native_sessions (
    token_hash bytea PRIMARY KEY REFERENCES sessions(token_hash) ON DELETE CASCADE
);

CREATE TABLE native_auth_attempts (
    id uuid PRIMARY KEY,
    provider text NOT NULL CHECK (provider IN ('apple','google','microsoft')),
    mode text NOT NULL CHECK (mode IN ('sign_in','link')),
    app_state text NOT NULL,
    challenge text NOT NULL,
    callback_identifier text NOT NULL CHECK (callback_identifier = 'pinvoyager'),
    user_id uuid REFERENCES users(id) ON DELETE CASCADE,
    provider_state text UNIQUE,
    started_at timestamptz,
    expires_at timestamptz NOT NULL,
    handoff_hash bytea,
    code_expires_at timestamptz,
    redeemed_at timestamptz
);
CREATE INDEX native_auth_attempts_expiry_idx ON native_auth_attempts(expires_at);

-- Keep the native origin on the provider attempt even after native expiry/cleanup.
-- An expired app flow must never fall through into browser sign-in.
ALTER TABLE auth_attempts ADD COLUMN native_attempt_id uuid;
