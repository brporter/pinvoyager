# Phase 8 — Discovery & polish

**Status:** not started
**Depends on:** phase 7
**Owner:** Claude

## Goal

The site works end to end for a stranger: they can find voyages without an account, browse a
creator's profile, and see a coherent product rather than a set of working endpoints. Account
deletion works. The application can be deployed.

The site will be hosted on a single box for now; no moderation, no photo reporting path. Sitemap
for search indexing is needed. No backup or retention policy at this time.

## Non-goals

- No social graph — no following, likes, or comments.
- No recommendation algorithm. Recency and simple ordering are enough.
- No native mobile app.
- No internationalization, unless promoted to a target.

## Targets

1. Discover screen — browse public voyages, paginated, per the phase 1 design.
2. Public profile — a user's voyages, visible without signing in.
3. Account settings — display name, avatar, default granularity, linked providers, with the
   ability to link and unlink. Unlinking the **last** provider must be refused; per D2 there
   is no other way back into the account.
4. Account deletion — a real one. Removes the user, identities, sessions, voyages, photo rows,
   and de-references stored files (respecting phase 4's reference counting so shared bytes
   are not pulled out from under another user).
5. Every empty, loading, and error state from the design implemented across every route.
6. A full responsive pass on real devices, not only a resized desktop browser.
7. Open Graph and Twitter card metadata for voyages, so a shared link previews with a photo.
   Voyages are public and sharing is the point.
8. Security headers: a CSP that actually constrains, plus HSTS, frame options, and referrer
   policy. MapLibre's worker usage interacts with CSP and needs attention rather than a
   blanket `unsafe-*`.
9. Production build and deployment: container image, tile archive placement, and backups for
   the database and the upload directory.
   **Migrations run as their own deploy step** (`sqlx migrate run`), not from application
   startup. Running them at boot races as soon as there is more than one instance, and two
   processes applying the same migration concurrently is a bad way to discover this.
10. Operational baseline: structured logs shipped somewhere, an uptime check, and an error
    budget for the tile handler under pan load.
11. Deployment settings that have to agree with each other, and silently do nothing when they
    do not:
    - `server.shutdown_timeout_secs` must be **shorter** than the orchestrator's grace period
      (Kubernetes `terminationGracePeriodSeconds`, default 30s; compose `stop_grace_period`).
      Longer and the SIGKILL lands mid-drain, making the graceful shutdown decorative.
    - Startup migrations need a config flag to disable them, so migrations can move to their
      own deploy step (target 9). Phase 2 has no such flag — add it here.
    - `SetRequestIdLayer` preserves a client-supplied `x-request-id` rather than overwriting
      it. Correct for cross-service propagation, but once a reverse proxy sits in front, make
      the trust boundary explicit: decide whether the edge strips or rewrites the header.

## Acceptance criteria

- **T3** — Attempting to unlink a sole remaining provider is refused with a clear explanation.
- **T4** — After deletion, the user's voyages 404, their sessions are invalid, and any file
  shared with another user's photo still serves correctly.
- **T6** — Verified on a real iOS device and a real Android device. Touch, sheet dragging, and
  the upload picker all behave — mobile file pickers in particular differ from desktop in ways
  that only show up on hardware.
- **T7** — A voyage URL pasted into a chat client renders a preview card with a photo.
- **T8** — The CSP report is clean on every route including voyage detail with the map live.
- **T9** — A deploy from a clean checkout produces a working site with migrations applied.

## Design references

- `design/screens/discover.html`
- `design/screens/profile.html`
- `design/screens/settings.html`
- `design/components/feedback.html`

## Notes

The moderation question above is the largest unaddressed risk in the current plan. Everything
else here is finishing work; that one is a missing capability. It is raised as an open
question rather than a target because how much is needed depends on how public the site
actually becomes, and building an admin console before launch may be premature. Not deciding
it at all is the failure mode.
