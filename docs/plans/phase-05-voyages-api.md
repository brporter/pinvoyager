# Phase 5 — Voyages API

**Status:** complete
**Depends on:** phase 4
**Owner:** Claude

## Goal

A signed-in user can create a voyage, attach photos to it, order them, set its location
granularity, and publish it. Anyone — signed in or not — can read a published voyage and get
back exactly the coordinate precision the owner chose. Granularity is set per-voyage. Granularity
semantics just lump photos together in a middle-cluster for the point. Deleting a voyage returns
the photos in that voyage to the library.

## Non-goals

- No frontend (phases 6–7).
- No comments, likes, follows, or any social layer.
- No search or discovery ranking (phase 8).
- No private or unlisted published voyages. Drafts remain owner-only; see D6.

## Targets

1. Migration: `voyages` and `voyage_photos`, with ordering and a granularity setting on the
   voyage.
2. `POST /api/voyages`, `PATCH /api/voyages/{id}`, `DELETE /api/voyages/{id}` — owner only.
3. `POST /api/voyages/{id}/photos` — attach photos the caller owns. Attaching a photo owned by
   someone else fails.
4. `PATCH /api/voyages/{id}/photos/order` — explicit ordering, defaulting to capture time.
5. Granularity applied on read: every coordinate the API emits for a voyage is transformed to
   that voyage's precision. **Exact coordinates never leave the server for a voyage set to
   anything coarser than exact** — including in list endpoints, embedded objects, and error
   messages.
6. `GET /api/voyages/{id}` — public, no auth required. Includes photos, their coordinates at
   the effective granularity, and the unplaced ones.
7. `GET /api/voyages` — paginated list, cursor-based.
8. Draft versus published state, so a voyage is not world-readable the instant it is created
   and half-populated.
9. A map-oriented endpoint returning just what the map needs — coordinates, photo ID,
   thumbnail URL — without the full photo payload, since phase 7 will fetch this for every
   pin on screen.

## Acceptance criteria

- **T3** — Attaching another user's photo returns 403 and attaches nothing.
- **T5** — With granularity set to ~10 km, **grep every JSON response body across every
  endpoint for the exact stored coordinate; it appears nowhere.** Changing granularity to
  exact and re-reading returns full precision. This is the phase's central check.
- **T5** — Rounding is stable: reading the same photo twice returns identical coordinates,
  not two different samples from a fuzz radius. Otherwise pins visibly jitter between loads
  and repeated sampling narrows the true location statistically.
- **T6** — An unauthenticated request reads a published voyage. A draft returns 404 to
  everyone but its owner — 404 rather than 403, so draft existence does not leak.
- **T7** — Pagination is stable across insertions during traversal.
- **T6/T9** — No N+1 queries. Loading a voyage with 48 photos issues a bounded number of
  queries, not one per photo. This is worth calling out specifically because `await` inside a
  loop *looks* concurrent and is not — it serializes every round trip, and the cost scales
  with photo count on the product's most-visited endpoint. Assert on query count in a test
  rather than trusting a read of the code.

## Design references

- `design/screens/voyage-detail.html`
- `design/screens/voyage-edit.html`
- `design/components/granularity.html`
- `design/flows/voyage-lifecycle.html`

## Open questions

- **The discovery cursor is not total, and silently drops rows.** `list_published` paginates on
  `published_at DESC` alone, which is not a unique ordering. With a strict `<`, two voyages
  sharing an exact `published_at` across a page boundary lose one of them — and they *will*
  share one, because `now()` is the transaction timestamp, so anything published in a single
  transaction gets an identical value. The fix is a composite `(published_at, id)` cursor with
  a matching `(published_at DESC, id DESC)` index. My contract's fault; T7's
  insertion-stability property does hold, but the tie case does not.
- **`reorder` cannot report "no such voyage, or not yours".** It returns `Result<(), _>` while
  every other owner-scoped mutation returns `Ok(None)`/`false`, so the handler has to establish
  ownership with a separate query — exactly the check-then-act shape the rest of the repository
  is arranged to avoid. Should return `Result<bool, _>`.
- **`attach_photos`'s count is ambiguous.** A result below `photo_ids.len()` means either "one
  of those is not yours" (a 403 per T3) or "one was already attached" (idempotent and fine),
  and the handler cannot tell which.
- **Nothing defends against a non-finite coordinate.** Postgres accepts `'NaN'` in a numeric
  column, and `Granularity::apply` carries it straight through — `clamp` propagates NaN, and
  `serde_json` renders a non-finite float as `null`. It does not panic and it does not leak,
  but the map endpoint would receive `null` where it expects a number. The right place to
  reject it is photo ingest, which knows what it read; confirm `photos::exif` range-checks
  before this is relied on.

## Notes

### What the leak sweep covers, and what it cannot

The sweep captures status, headers and body from voyage detail, the discovery list, the map
endpoint, and a 404 error path — for three callers each: anonymous, a stranger with a session,
and the owner. Granularity is a property of the voyage, not of the reader, so all three must
agree.

It searches for five decimal places as a *prefix*, so a truncated or float-round-tripped leak
still trips it, then flips the voyage to `exact` and asserts the full digits do come back — an
implementation that simply dropped all coordinates would otherwise pass. The sweep is
mutation-tested: setting the fixture to `Exact` makes it fail on the first endpoint.

**Three paths it does not cover**, worth knowing rather than assuming:

1. **Served image bytes.** `GET /api/photos/{id}/{variant}` could carry the original GPS EXIF
   and no JSON grep would ever see it. This is covered — but by phase 4's `exiftool` check on
   the actual served bytes, not by this file. The two halves of D6 are verified by two
   different mechanisms, and neither alone is sufficient.
2. **Mutation-endpoint error bodies.** Only GET error paths are swept; an echoing 400 from
   `POST /api/voyages/{id}/photos` would be missed.
3. **`GET /api/photos` and `/api/photos/{id}` return exact coordinates to the photograph's
   owner**, deliberately — a user's own library is not the public surface D6 governs. Written
   out as an explicit exemption in the test rather than silently skipped, on the same principle
   as the `PERMITTED` list in `integration.rs`, so it cannot outlive its reason.

Query counting deserves a note: `pg_stat_statements` is not available, sqlx's logging depends
on which subscriber won, and timing is not a count. So the test puts a **TCP proxy between the
pool and PostgreSQL and parses the v3 wire protocol**, counting frontend `Execute`/`Query`
frames and excluding `Parse`/`Bind`/`Describe` so a cold statement cache does not read as an
N+1. It is self-calibrating — 48 deliberate round trips must be counted as exactly 48 — so a
counter that stopped parsing cannot report a flatteringly small number. Measured: **two
statements for a 4-photo voyage and two for a 48-photo one.**

### The longitude correction, measured

Recorded because the failure mode is silent and points the wrong way.

A degree of latitude is ~111.32 km everywhere; a degree of longitude shrinks with
`cos(latitude)`. A fixed-degree grid therefore produces cells *narrower* than advertised as you
go north — the product would promise less precision than it delivers, which is the dangerous
direction for a privacy feature to be wrong in.

Measured width of a `Km10` cell, by sweeping longitudes and watching where the output jumps:

| latitude | with the correction | without it |
|---|---|---|
| 0° | 10.000 km | 10.00 km |
| 45° | 9.993 km | 7.07 km |
| 60° | 9.989 km | 5.00 km |
| 70° | 10.011 km | 3.42 km |

The scaling is computed from the **rounded** latitude, not the raw one, so which cell a
longitude falls into does not depend on the value being hidden — and two photos in one cell
still agree on an answer.

Near the poles the correction runs to infinity, so it is frozen at 85°: the widening factor
there is already 11.5×, and the northernmost inhabited place on Earth is around 82.5°N. Above
that the latitude axis alone supplies the ambiguity.

The implementation was mutation-tested against the three ways it goes wrong — dropping `cos`,
using the raw latitude, and flooring instead of centring — and each produces failures.

T5 is the whole privacy model. Granularity is enforced in exactly one place — the
serialization boundary — because enforcing it per endpoint guarantees that the one endpoint
someone forgets becomes the leak.

Concretely: a `Coordinate` response newtype in its own module with **private fields**, whose
only constructor takes a granularity argument. Rust's module privacy makes this genuinely
unforgeable rather than merely conventional — there is no way to get a latitude into a response
body without having stated a precision, and the type system enforces it at compile time instead
of relying on everyone remembering. The row type carrying raw coordinates should not implement
`Serialize` at all.

Note how this interacts with phase 4's open question: if exact coordinates are discarded at
ingest, T5 is trivially satisfied but granularity can never be loosened afterward.
