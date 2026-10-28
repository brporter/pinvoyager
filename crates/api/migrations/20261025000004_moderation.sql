CREATE TABLE account_blocks (
    account_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, blocked_id),
    CHECK (account_id <> blocked_id)
);
ALTER TABLE voyages ADD COLUMN moderation_held boolean NOT NULL DEFAULT false;
CREATE TABLE content_reports (
    id uuid PRIMARY KEY,
    reporter_id uuid REFERENCES users(id) ON DELETE SET NULL,
    target_owner_id uuid REFERENCES users(id) ON DELETE SET NULL,
    voyage_id uuid REFERENCES voyages(id) ON DELETE SET NULL,
    target text NOT NULL,
    reason text NOT NULL,
    details text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    status text NOT NULL DEFAULT 'open' CHECK (status IN ('open','held','resolved')),
    moderator_id uuid REFERENCES users(id) ON DELETE SET NULL,
    moderator_note text,
    resolved_at timestamptz
);
CREATE INDEX content_reports_queue ON content_reports(id DESC);
CREATE INDEX content_reports_rate ON content_reports(reporter_id,created_at);
-- The HTTP paths return a safe conflict before this final invariant is checked.
ALTER TABLE voyages ADD CONSTRAINT held_voyages_are_private CHECK (NOT moderation_held OR NOT published);
