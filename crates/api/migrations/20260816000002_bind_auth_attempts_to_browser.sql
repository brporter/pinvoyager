-- Bind each authorization attempt to the browser that started it.
--
-- Closes a link-CSRF hole that `link_user_id` alone does not:
--
--   1. The attacker signs in to their own account and starts a link flow,
--      getting a provider URL whose `state` is bound to an attempt with
--      link_user_id = attacker.
--   2. They do not follow it. They lure a victim into clicking it.
--   3. The victim is signed in at the provider, sees an ordinary consent
--      screen, and approves.
--   4. The provider redirects the VICTIM's browser to our callback carrying
--      the ATTACKER's state.
--   5. Without this column we consume the attempt, read link_user_id, and
--      attach the victim's provider identity to the attacker's account.
--
-- The victim gains an attacker-controlled account they did not ask for, and
-- every future sign-in they make with that provider lands in it. Social-only
-- sign-in (D2) is exactly the shape where this bites, because there is no
-- password step anywhere to interrupt it.
--
-- The fix is to require the callback to arrive in the same browser that began
-- the flow. `/auth/{provider}/start` mints a high-entropy binding value, sets
-- it as a cookie, and stores its hash here; the callback recomputes the hash
-- from the cookie it receives and must match. The attacker cannot set a cookie
-- in the victim's browser, so the victim's callback carries either no binding
-- cookie or their own — never the attacker's.
--
-- Note this is deliberately NOT the session cookie. Matching the callback's
-- *session* against link_user_id would seem simpler and would break Apple
-- outright: its callback is a cross-site form POST, and a SameSite=Lax cookie
-- is not sent on one. The binding cookie is SameSite=None so that it survives
-- that POST, which is safe precisely because it is a single-use opaque value
-- that confers nothing on its own.

-- Adding a NOT NULL column with no default fails against existing rows, and
-- there is no honest value to backfill — an attempt started before this
-- migration has no binding cookie in anyone's browser. Discarding them is
-- correct rather than merely convenient: the rows live at most ten minutes, and
-- the worst case is that someone mid-sign-in sees "starting again normally
-- works", which is true.
DELETE FROM auth_attempts;

ALTER TABLE auth_attempts
    -- SHA-256 of the binding value, never the value itself — same reasoning as
    -- sessions.token_hash. The raw value lives only in the browser's cookie.
    ADD COLUMN binding_hash bytea NOT NULL;
