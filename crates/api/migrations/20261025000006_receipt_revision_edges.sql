-- Historical receipts without a known predecessor remain usable for identity recovery,
-- but cannot prove that a client has observed intervening voyage changes.
ALTER TABLE native_operations ADD COLUMN previous_revision bigint;
ALTER TABLE upload_receipts ADD COLUMN previous_revision bigint;
