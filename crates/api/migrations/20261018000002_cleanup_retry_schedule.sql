-- Failed removals wait before retrying, allowing other due records to progress.
ALTER TABLE photo_cleanup ADD COLUMN next_attempt_at timestamptz NOT NULL DEFAULT now();
CREATE INDEX photo_cleanup_next_attempt_idx ON photo_cleanup(next_attempt_at, created_at, content_hash);
