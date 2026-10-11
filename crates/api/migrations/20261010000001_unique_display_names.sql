-- D14: system prefixes, current handles, and historical aliases share one namespace.
CREATE TABLE profile_names (
    name text PRIMARY KEY CHECK (name = lower(name)),
    -- NULL means a system name or a deleted account's permanently reserved name.
    user_id uuid REFERENCES users(id) ON DELETE SET NULL
);
INSERT INTO profile_names (name) VALUES
('api'), ('auth'), ('u'), ('voyages'), ('upload'), ('settings'), ('signin'),
('discover'), ('health'), ('ready'), ('onboarding'), ('admin'), ('assets'),
('static'), ('app'), ('account'), ('accounts'), ('users'), ('user'), ('profile'),
('profiles'), ('photos'), ('images'), ('login'), ('logout'), ('signup'), ('register'),
('search'), ('help'), ('support'), ('about'), ('contact'), ('terms'), ('privacy'),
('legal'), ('robots'), ('sitemap'), ('favicon'), ('manifest'), ('status'), ('src');

-- Preserve valid unique legacy handles. Provider labels are not URL handles;
-- all participants in an ambiguous name must choose, rather than assigning a winner.
UPDATE users SET display_name = lower(trim(display_name)) WHERE display_name IS NOT NULL;
UPDATE users SET display_name = NULL, updated_at = now()
WHERE display_name IS NOT NULL AND (
    display_name !~ '^[a-z][a-z0-9-]{1,28}[a-z0-9]$'
    OR display_name IN (SELECT name FROM profile_names)
    OR display_name IN (SELECT display_name FROM users WHERE display_name IS NOT NULL
                         GROUP BY display_name HAVING count(*) > 1)
);
CREATE UNIQUE INDEX users_display_name_key ON users (lower(display_name));
INSERT INTO profile_names (name, user_id)
SELECT display_name, id FROM users WHERE display_name IS NOT NULL;
