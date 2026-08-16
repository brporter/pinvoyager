-- Voyages: a set of photographs on a map.
--
-- Two decisions from the phase 5 plan are encoded here:
--
-- * **Granularity is per voyage**, not per photo. One setting governs every
--   coordinate the voyage emits, which is also what makes the serialization
--   boundary a single place rather than a per-photo decision repeated
--   everywhere.
--
-- * **Deleting a voyage returns its photos to the library.** The join table
--   cascades; the photos themselves do not. Someone tidying up a draft is not
--   asking to destroy the photographs in it, and the alternative — deleting
--   the images — is the kind of surprise people only discover once.

CREATE TABLE voyages (
    id           uuid        PRIMARY KEY,
    owner_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    title        text        NOT NULL,
    description  text,

    -- Draft until deliberately published (T8). A voyage is world-readable the
    -- moment it is published (D6), so it must not be readable while it is
    -- still being assembled — and "published" has to be a decision, not the
    -- default state of a half-built row.
    published    boolean     NOT NULL DEFAULT false,

    -- One of the four precision steps. Text with a CHECK rather than a
    -- Postgres enum, for the same reason as identities.provider: the set is a
    -- product decision and adding a step should not need ALTER TYPE.
    --
    -- The values name distances rather than encoding them, so the mapping from
    -- a step to a grid size lives in code where it can be tested.
    granularity  text        NOT NULL DEFAULT 'exact'
        CHECK (granularity IN ('exact', 'km1', 'km10', 'km50')),

    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    -- Null until first published. Distinct from created_at because the
    -- discovery feed orders by when something became visible, not by when its
    -- author started drafting it.
    published_at timestamptz
);

-- The owner's list, and the ownership check on every mutation.
CREATE INDEX voyages_owner_id_idx ON voyages (owner_id, id DESC);

-- The discovery feed: published only, newest first. Partial, because a draft
-- is never in it and there is no reason to carry drafts in the index.
CREATE INDEX voyages_published_idx ON voyages (published_at DESC)
    WHERE published;

-- Which photos are in a voyage, and in what order.
CREATE TABLE voyage_photos (
    voyage_id uuid    NOT NULL REFERENCES voyages (id) ON DELETE CASCADE,
    photo_id  uuid    NOT NULL REFERENCES photos (id) ON DELETE CASCADE,

    -- Explicit ordering (T4). Defaults to capture time at insert, but stored
    -- rather than derived so a manual reorder is possible without a second
    -- mechanism — and so a voyage of photos that all lack a capture time still
    -- has a stable order.
    position  integer NOT NULL,

    added_at  timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (voyage_id, photo_id)
);

-- Reading a voyage walks this in order. The primary key already covers
-- (voyage_id, photo_id); this one covers (voyage_id, position) so the ordered
-- read is an index scan rather than a sort.
CREATE INDEX voyage_photos_order_idx ON voyage_photos (voyage_id, position);

-- "Which voyages is this photo in?" — needed when a photo is deleted, and by
-- the library view to show whether a photo is already used.
CREATE INDEX voyage_photos_photo_id_idx ON voyage_photos (photo_id);
