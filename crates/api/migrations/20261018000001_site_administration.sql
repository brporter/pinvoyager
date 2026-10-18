-- Roles default to ordinary, enabled accounts. Bootstrap the first administrator
-- explicitly; provider claims and public profile updates cannot grant this role.
ALTER TABLE users ADD COLUMN is_site_admin boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN suspended boolean NOT NULL DEFAULT false;
