# Phase 5 — Voyages API

**Status:** not started
**Depends on:** phase 4
**Owner:** Claude

## Goal

A signed-in user can create a voyage, attach photos to it, order them, set its location
granularity, and publish it. Anyone — signed in or not — can read a published voyage and get
back exactly the coordinate precision the owner chose.

## Non-goals

- No frontend (phases 6–7).
- No comments, likes, follows, or any social layer.
- No search or discovery ranking (phase 8).
- No visibility levels. All voyages are public per D6.

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
  loop *looks* concurrent and is not — it serialises every round trip, and the cost scales
  with photo count on the product's most-visited endpoint. Assert on query count in a test
  rather than trusting a read of the code.

## Design references

- `design/screens/voyage-detail.html`
- `design/screens/voyage-edit.html`
- `design/components/granularity.html`
- `design/flows/voyage-lifecycle.html`

## Open questions

- **Granularity scope.** Per voyage, per photo, or a user-level default that a voyage
  inherits and can override? Currently drawn as per voyage with a user-level default. Per
  photo is more flexible but considerably more UI.
- **Granularity semantics.** Rounding to a grid (deterministic, but reveals grid membership
  and clusters pins onto grid intersections) versus a fixed per-photo offset derived from a
  secret and the photo ID (looks natural, stable per T5, requires care that repeated
  observation cannot average it out). Grid rounding is simpler and probably right.
- Whether deleting a voyage deletes its photos or returns them to the library. Returning
  them is more forgiving.

## Notes

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
