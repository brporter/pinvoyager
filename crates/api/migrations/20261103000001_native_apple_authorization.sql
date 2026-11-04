-- Native Apple authorization is bound to a server nonce instead of a browser cookie.
-- started_at prevents these attempts from entering the browser bootstrap endpoint.
ALTER TABLE native_auth_attempts ADD COLUMN apple_nonce text;
