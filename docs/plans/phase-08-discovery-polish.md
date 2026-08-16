# Phase 8 — Discovery & polish

**Status:** not started
**Depends on:** phase 7
**Owner:** Claude

## Goal

The site works end to end for a stranger: they can find voyages without an account, browse a
creator's profile, and see a coherent product rather than a set of working endpoints. Account
deletion works. The application can be deployed.

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

## Open questions

- **Hosting target.** A single box fits the architecture well — files on local disk and a
  100+ GB tile archive both favor it. Object storage and a CDN would change phase 4's storage
  layer and phase 7's tile serving, so if that is the intent, it should be decided before
  phase 4 rather than here.
- **Moderation.** Public photo uploads from arbitrary accounts attract abuse. There is
  currently no reporting mechanism, no takedown path, and no admin surface. That is a real
  gap for a public site and probably deserves its own phase.
- Whether a sitemap and search-engine indexing are wanted. Public voyages suggest yes.
- Backup and retention policy for uploaded originals.

## Notes

The moderation question above is the largest unaddressed risk in the current plan. Everything
else here is finishing work; that one is a missing capability. It is raised as an open
question rather than a target because how much is needed depends on how public the site
actually becomes, and building an admin console before launch may be premature. Not deciding
it at all is the failure mode.
