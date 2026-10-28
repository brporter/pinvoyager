CREATE TABLE publication_intents (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    voyage_id uuid NOT NULL REFERENCES voyages(id) ON DELETE CASCADE,
    metadata_digest text NOT NULL,
    photos jsonb NOT NULL,
    tracked_revision bigint NOT NULL,
    expires_at timestamptz NOT NULL,
    status text NOT NULL CHECK (status IN ('waiting_for_uploads','processing','published','canceled','expired','needs_review','failed')),
    reason text
);
CREATE UNIQUE INDEX one_active_publication ON publication_intents(voyage_id)
WHERE status IN ('waiting_for_uploads','processing');
CREATE INDEX pending_publication_scan ON publication_intents(id)
WHERE status IN ('waiting_for_uploads','processing');

-- Even an explicit no-op unpublish invalidates earlier consent and advances revision.
DROP TRIGGER voyage_content_revision ON voyages;
CREATE OR REPLACE FUNCTION advance_voyage_content_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.revision := OLD.revision + 1;
    UPDATE publication_intents SET status='needs_review',reason='Voyage details or visibility changed.'
     WHERE voyage_id=OLD.id AND status IN ('waiting_for_uploads','processing')
       AND id::text IS DISTINCT FROM current_setting('pinvoyager.expected_intent',true);
    UPDATE publication_intents SET tracked_revision=NEW.revision
     WHERE voyage_id=OLD.id AND status IN ('waiting_for_uploads','processing')
       AND id::text=current_setting('pinvoyager.expected_intent',true);
    RETURN NEW;
END $$;
CREATE TRIGGER voyage_content_revision BEFORE UPDATE OF title,description,granularity,published ON voyages
FOR EACH ROW EXECUTE FUNCTION advance_voyage_content_revision();

CREATE FUNCTION record_photo_revision(voyage uuid) RETURNS void LANGUAGE plpgsql AS $$
DECLARE next_revision bigint;
BEGIN
    UPDATE voyages SET revision=revision+1 WHERE id=voyage RETURNING revision INTO next_revision;
    UPDATE publication_intents SET status='needs_review',reason='The photo list or order changed.'
     WHERE voyage_id=voyage AND status IN ('waiting_for_uploads','processing')
       AND id::text IS DISTINCT FROM current_setting('pinvoyager.expected_intent',true);
    UPDATE publication_intents SET tracked_revision=next_revision
     WHERE voyage_id=voyage AND status IN ('waiting_for_uploads','processing')
       AND id::text=current_setting('pinvoyager.expected_intent',true);
END $$;

CREATE OR REPLACE FUNCTION photo_content_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND (NEW.position,NEW.voyage_id) IS NOT DISTINCT FROM (OLD.position,OLD.voyage_id) THEN RETURN NULL; END IF;
    IF TG_OP<>'INSERT' THEN PERFORM record_photo_revision(OLD.voyage_id); END IF;
    IF TG_OP='INSERT' OR (TG_OP='UPDATE' AND NEW.voyage_id<>OLD.voyage_id) THEN PERFORM record_photo_revision(NEW.voyage_id); END IF;
    RETURN NULL;
END $$;
