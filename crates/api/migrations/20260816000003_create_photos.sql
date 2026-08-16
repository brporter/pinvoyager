-- Photos: the uploader's library. Voyage association is phase 5.
--
-- D6/D7 govern this table. Two things it deliberately does and does not do:
--
-- * It stores EXACT coordinates. Granularity is applied at serialization
--   (phase 5), not at ingest. That keeps the precision available for a later
--   change of granularity and for the future "adjust location" feature, at the
--   cost of holding exact positions in the database. Rounding at ingest would
--   make phase 5's central check trivially true and the decision irreversible.
--
-- * It stores no filename from the client. Storage paths are derived from a
--   hash of the content, which removes path traversal as a category rather
--   than defending against it.

CREATE TABLE photos (
    id             uuid        PRIMARY KEY,
    owner_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- SHA-256 of the ORIGINAL uploaded bytes, hex-encoded. Both the storage
    -- path and the dedup key.
    --
    -- Not unique: two users uploading identical bytes get two rows sharing one
    -- file. That is the point of content addressing, and it is why deletion has
    -- to be reference-counted — an unqualified unlink would remove a file
    -- another user's photo still points at.
    content_hash   text        NOT NULL,

    -- Bytes and pixel dimensions of the original, recorded after validation so
    -- a later reader never has to re-open the file to answer "how big".
    byte_size      bigint      NOT NULL CHECK (byte_size > 0),
    width          integer     NOT NULL CHECK (width > 0),
    height         integer     NOT NULL CHECK (height > 0),

    -- Detected from magic bytes, never from the filename or the client's
    -- Content-Type. Determines which decoder handles it.
    original_format text       NOT NULL
        CHECK (original_format IN ('jpeg', 'png', 'webp', 'heic')),

    -- EXIF capture time. Null is ordinary: screenshots and scans have none.
    captured_at    timestamptz,

    -- EXACT coordinates from EXIF, or null. Null is the common case, not an
    -- error — screenshots, scans, and any camera with location services off
    -- carry no GPS at all.
    --
    -- Stored as numeric rather than float8 so a coordinate round-trips without
    -- drift; at these magnitudes float8 is fine in practice, but "the number we
    -- were given" is a better thing to hold than "a number very close to it"
    -- when the whole privacy model is about controlling precision.
    latitude       numeric(9, 6),
    longitude      numeric(9, 6),

    -- Either both coordinates or neither. A half-placed photo has no meaning
    -- and would force every reader to handle a state that cannot occur.
    CONSTRAINT photos_coordinates_complete
        CHECK ((latitude IS NULL) = (longitude IS NULL)),

    -- Variants are generated off the request path, so a photo exists before its
    -- derived images do. Readers must handle 'pending'.
    variant_status text        NOT NULL DEFAULT 'pending'
        CHECK (variant_status IN ('pending', 'ready', 'failed')),

    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);

-- "My library", newest first. id is uuid v7, so ordering by it is chronological
-- and the index serves both the filter and the sort.
CREATE INDEX photos_owner_id_idx ON photos (owner_id, id DESC);

-- Reference counting on delete: "does any other row still use this content?"
CREATE INDEX photos_content_hash_idx ON photos (content_hash);

-- The worker's queue scan.
CREATE INDEX photos_variant_status_idx ON photos (variant_status)
    WHERE variant_status = 'pending';
