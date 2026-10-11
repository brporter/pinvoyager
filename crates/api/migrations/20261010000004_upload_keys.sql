-- One request key and file ordinal identify a retry within its voyage.
ALTER TABLE photos ADD COLUMN upload_key text;
CREATE UNIQUE INDEX photos_upload_key ON photos(voyage_id, upload_key)
    WHERE upload_key IS NOT NULL;
