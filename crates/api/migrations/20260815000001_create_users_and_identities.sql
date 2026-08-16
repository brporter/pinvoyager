-- Initial schema: users and identities (D3).
--
-- D1: this migration stores no credential of any kind. There is no password,
-- no hash, no provider access or refresh token, and no session material. The
-- provider's ID token is verified on receipt and discarded; what survives is
-- only the fact that a given provider vouched for a given subject. A future
-- migration adding a credential column to either table is a mistake, not a
-- feature.

-- The account. Outlives any single provider, which is the whole reason it is
-- separate from `identities`: a user can drop Google and keep their voyages.
CREATE TABLE users (
    id            uuid        PRIMARY KEY,
    -- Both nullable because providers differ in what they return, and Apple
    -- returns a name only on the very first authorization (D1) — so "we do not
    -- know this user's name" is a state the schema has to be able to represent.
    display_name  text,
    avatar_url    text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    -- Maintained by the application rather than a trigger: every write to this
    -- table goes through the repository layer, and a trigger would hide the
    -- update from anyone reading the query.
    updated_at    timestamptz NOT NULL DEFAULT now()
);

-- Deliberately no `email` column. Nothing in the product needs one yet, and
-- D3 is explicit that two providers can return the same address for the same
-- human while not every provider verifies ownership — so an email column
-- invites exactly the matching logic that is an account-takeover vector.
-- Adding it later is a migration; removing it after code depends on it is not.

-- One row per linked provider. This is the mapping (provider, subject) -> user.
CREATE TABLE identities (
    id           uuid        PRIMARY KEY,
    -- ON DELETE CASCADE: deleting the account must not leave orphaned identity
    -- rows behind, because a surviving row would let the provider subject
    -- resolve to a user that no longer exists.
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Stored as text rather than a Postgres enum type. A Postgres enum needs
    -- ALTER TYPE to add a value and cannot remove one, and the set of providers
    -- is a product decision (D2), not a storage one. The CHECK below keeps the
    -- column honest without that rigidity.
    provider     text        NOT NULL,
    -- The provider's stable subject identifier: Google/Microsoft/Apple `sub`,
    -- Facebook's app-scoped ID. Opaque to us — never parsed, only compared.
    subject      text        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),

    -- The four providers of D2. A row with any other value could only come from
    -- a bug or a manual edit, and would fail to decode into the Rust `Provider`
    -- enum anyway — better to reject it at write time, where the cause is still
    -- visible, than at read time in an unrelated request.
    CONSTRAINT identities_provider_check
        CHECK (provider IN ('google', 'microsoft', 'apple', 'facebook'))
);

-- Required by D3, and load-bearing rather than merely tidy: it is what stops a
-- single provider identity from being attached to two different accounts. If a
-- race, a retried callback, or a buggy link flow tries to claim an already
-- linked (provider, subject), the database refuses it instead of silently
-- splitting one human across two accounts. It also serves the sign-in lookup,
-- which is keyed on exactly this pair.
CREATE UNIQUE INDEX identities_provider_subject_key
    ON identities (provider, subject);

-- Serves "show me my linked providers" in account settings — which D2 makes a
-- real screen rather than a nicety, since a user who loses access to every
-- linked provider has no recovery path we control.
CREATE INDEX identities_user_id_idx
    ON identities (user_id);
