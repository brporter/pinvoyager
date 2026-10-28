ALTER TABLE native_auth_attempts DROP CONSTRAINT native_auth_attempts_mode_check;
ALTER TABLE native_auth_attempts ADD CONSTRAINT native_auth_attempts_mode_check CHECK (mode IN ('sign_in','link','reauth_delete'));
CREATE TABLE deletion_proofs (
    token_hash bytea PRIMARY KEY,
    account_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider text NOT NULL,
    subject_hash bytea NOT NULL,
    expires_at timestamptz NOT NULL,
    used boolean NOT NULL DEFAULT false
);
CREATE TABLE account_deletions (
    id uuid PRIMARY KEY,
    account_id uuid REFERENCES users(id) ON DELETE SET NULL,
    request_hash bytea NOT NULL,
    status_hash bytea NOT NULL,
    apple_subject_hash bytea,
    state text NOT NULL CHECK (state IN ('prepared','deleting','complete')),
    expires_at timestamptz NOT NULL,
    retain_until timestamptz NOT NULL DEFAULT now()+interval '30 days',
    hashes text[] NOT NULL DEFAULT '{}'
);
CREATE INDEX account_deletions_cleanup ON account_deletions(state) WHERE state='deleting';
