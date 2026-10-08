-- D11: every photo belongs to exactly one voyage with the same owner.
ALTER TABLE photos ADD COLUMN voyage_id uuid;
ALTER TABLE photos ADD COLUMN position integer NOT NULL DEFAULT 1;

-- Preserve the first membership on the original record.
WITH first_membership AS (
    SELECT DISTINCT ON (photo_id) photo_id, voyage_id, position
    FROM voyage_photos ORDER BY photo_id, added_at, voyage_id
)
UPDATE photos p SET voyage_id = m.voyage_id, position = m.position
FROM first_membership m WHERE p.id = m.photo_id;

-- Preserve additional memberships as distinct records sharing stored bytes.
INSERT INTO photos (id, owner_id, voyage_id, position, content_hash, byte_size,
                    width, height, original_format, captured_at, latitude,
                    longitude, variant_status, created_at, updated_at)
SELECT gen_random_uuid(), p.owner_id, vp.voyage_id, vp.position, p.content_hash,
       p.byte_size, p.width, p.height, p.original_format, p.captured_at,
       p.latitude, p.longitude, p.variant_status, p.created_at, p.updated_at
FROM photos p JOIN voyage_photos vp ON vp.photo_id = p.id
WHERE vp.voyage_id <> p.voyage_id;

-- Recover unattached uploads into a private draft for each owner.
CREATE TEMP TABLE recovered_voyages ON COMMIT DROP AS
SELECT DISTINCT owner_id, gen_random_uuid() AS voyage_id
FROM (SELECT DISTINCT owner_id FROM photos WHERE voyage_id IS NULL) owners;
INSERT INTO voyages (id, owner_id, title)
SELECT voyage_id, owner_id, 'Recovered photos' FROM recovered_voyages;
WITH ranked AS (
    SELECT p.id, r.voyage_id,
           row_number() OVER (PARTITION BY p.owner_id ORDER BY p.captured_at NULLS LAST, p.id)::integer AS position
    FROM photos p JOIN recovered_voyages r ON r.owner_id = p.owner_id
    WHERE p.voyage_id IS NULL
)
UPDATE photos p SET voyage_id = r.voyage_id, position = r.position
FROM ranked r WHERE p.id = r.id;

ALTER TABLE voyages ADD CONSTRAINT voyages_id_owner_key UNIQUE (id, owner_id);
ALTER TABLE photos ALTER COLUMN voyage_id SET NOT NULL;
ALTER TABLE photos ADD CONSTRAINT photos_voyage_owner_fkey
    FOREIGN KEY (voyage_id, owner_id) REFERENCES voyages (id, owner_id) ON DELETE CASCADE;
CREATE INDEX photos_voyage_order_idx ON photos (voyage_id, position, id);
DROP TABLE voyage_photos;
-- Compatibility projection for existing map/discovery queries; no independent membership.
CREATE VIEW voyage_photos AS
SELECT voyage_id, id AS photo_id, position, created_at AS added_at FROM photos;
