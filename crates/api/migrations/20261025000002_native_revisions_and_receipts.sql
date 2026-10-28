ALTER TABLE voyages ADD COLUMN revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0);

CREATE TABLE native_operations (
    account_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    id uuid NOT NULL,
    request_hash bytea NOT NULL,
    state text NOT NULL CHECK (state IN ('in_progress','applied','rejected','deleted')),
    target_id uuid,
    voyage_id uuid,
    resource_kind text,
    result_revision bigint,
    response_status smallint CHECK (response_status BETWEEN 200 AND 599),
    response_body bytea,
    PRIMARY KEY(account_id,id)
);
CREATE INDEX native_operations_voyage_idx ON native_operations(voyage_id);

CREATE TABLE upload_receipts (
    account_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    voyage_id uuid NOT NULL,
    upload_key text NOT NULL,
    state text NOT NULL CHECK (state IN ('accepted','canceled','deleted')),
    photo_id uuid,
    content_hash text,
    revision bigint,
    PRIMARY KEY(account_id,voyage_id,upload_key)
);
CREATE INDEX upload_receipts_photo_idx ON upload_receipts(photo_id);

CREATE FUNCTION advance_voyage_content_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (NEW.title,NEW.description,NEW.granularity,NEW.published)
        IS DISTINCT FROM (OLD.title,OLD.description,OLD.granularity,OLD.published) THEN
        NEW.revision := OLD.revision + 1;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER voyage_content_revision BEFORE UPDATE ON voyages
FOR EACH ROW EXECUTE FUNCTION advance_voyage_content_revision();

CREATE FUNCTION photo_content_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND (NEW.position,NEW.voyage_id) IS NOT DISTINCT FROM (OLD.position,OLD.voyage_id) THEN
        RETURN NULL;
    END IF;
    IF TG_OP<>'INSERT' THEN
        UPDATE voyages SET revision=revision+1 WHERE id=OLD.voyage_id;
    END IF;
    IF TG_OP='INSERT' OR (TG_OP='UPDATE' AND NEW.voyage_id<>OLD.voyage_id) THEN
        UPDATE voyages SET revision=revision+1 WHERE id=NEW.voyage_id;
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER photo_content_revision AFTER INSERT OR DELETE OR UPDATE OF position,voyage_id ON photos
FOR EACH ROW EXECUTE FUNCTION photo_content_revision();

CREATE FUNCTION retain_deleted_native_receipts() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_TABLE_NAME='voyages' THEN
        UPDATE native_operations SET state='deleted' WHERE voyage_id=OLD.id AND state='applied';
        UPDATE upload_receipts SET state='deleted' WHERE voyage_id=OLD.id AND state='accepted';
    ELSE
        UPDATE native_operations SET state='deleted' WHERE resource_kind='photo' AND target_id=OLD.id AND state='applied';
        UPDATE upload_receipts SET state='deleted' WHERE photo_id=OLD.id AND state='accepted';
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER retain_voyage_receipts AFTER DELETE ON voyages
FOR EACH ROW EXECUTE FUNCTION retain_deleted_native_receipts();
CREATE TRIGGER retain_photo_receipts AFTER DELETE ON photos
FOR EACH ROW EXECUTE FUNCTION retain_deleted_native_receipts();
