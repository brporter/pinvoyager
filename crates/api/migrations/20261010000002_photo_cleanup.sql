-- Deletion commits before filesystem cleanup. Retain work across process exits
-- and storage failures; the worker rechecks references under the content lock.
CREATE TABLE photo_cleanup (
    content_hash text PRIMARY KEY,
    created_at timestamptz NOT NULL DEFAULT now()
);
