-- Remove the retired provider's links and pending attempts. Accounts, voyages,
-- and sessions remain available through any retained identities or sessions.
DELETE FROM auth_attempts WHERE provider = 'facebook';
DELETE FROM identities WHERE provider = 'facebook';

ALTER TABLE identities DROP CONSTRAINT identities_provider_check;
ALTER TABLE identities ADD CONSTRAINT identities_provider_check
    CHECK (provider IN ('google', 'microsoft', 'apple'));

ALTER TABLE auth_attempts DROP CONSTRAINT auth_attempts_provider_check;
ALTER TABLE auth_attempts ADD CONSTRAINT auth_attempts_provider_check
    CHECK (provider IN ('google', 'microsoft', 'apple'));
