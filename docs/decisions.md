# Decisions

Decisions that are settled. Later phases build on these rather than reopening them.

If you want to change one, edit it here first — this file outranks any plan document, and a
phase plan that contradicts it is wrong.

Each entry records the decision and *why*, because the reasoning is what tells you whether a
future change invalidates it.

---

## D1 — Authentication is delegated; no credentials are stored

OIDC/OAuth2 implemented directly in Rust against the four providers. The application stores
**no credentials of any kind**: no passwords, no password hashes, no long-lived provider
access or refresh tokens.

The provider's ID token is verified on receipt and then discarded. What persists is only the
fact of the identity — which provider, and that provider's stable subject identifier.

**Why:** credentials that are never stored cannot leak. This removes password reset flows,
hashing parameter choices, credential-stuffing exposure, and the breach-notification surface
that comes with holding secrets.

**What "verify" means here** — this is a security requirement, not an implementation detail,
and it is easy to get wrong by merely *decoding* a token:

- Fetch and cache each provider's JWKS; validate the ID token's signature against it
- Validate `iss`, `aud`, and `exp`
- Configure OIDC issuers with HTTP or HTTPS URLs that include a host; plain HTTP is for local test providers, while deployed providers use HTTPS.
- Use PKCE on the authorization code exchange
- Generate and check `state` (CSRF) and `nonce` (replay) on every attempt
- Facebook is **not** OIDC-conformant. It has no ID token to verify. It requires an OAuth2
  code exchange followed by a Graph API call, with the returned app-scoped ID as the subject.
  Budget for it being different from the other three.
- Apple returns the user's name **only on the very first authorization**, in the form-POST
  body, never again. If it is not captured at that moment it is gone permanently.

## D2 — Social sign-in only; first sign-in is signup

Google, Microsoft, Apple, Facebook. There is no email/password path and no magic-link path.
A successful first sign-in creates the account implicitly; there is no separate registration
screen.

**Why:** it follows from D1. Any self-owned credential path reintroduces the storage and
recovery surface that D1 exists to avoid.

**Consequence to design for:** if all of a user's providers become inaccessible to them,
there is no recovery path we control. Account settings must make linked providers visible and
should encourage linking a second one.

## D3 — Users are first-class; identities are separate

Two tables:

- `users` — the account. Identity, display name, avatar URL, settings. Owns voyages.
- `identities` — one row per linked provider, mapping `(provider, subject) → user_id`.
  Unique on `(provider, subject)`.

**Why:** voyages need a stable owner that outlives any single provider. Separating identities
lets one person link Google and Apple to the same account, and lets them drop a provider
without losing their work. Keying ownership directly off a provider subject would make
"sign in with Apple instead of Google" silently produce a second, unrelated account.

Neither table holds a credential. D1 still holds.

**Consequence to design for:** two providers can return the same email address for the same
human. Matching on email alone is an account-takeover vector, since not every provider
verifies email ownership. Linking must be an explicit, authenticated action by a user already
signed in — never an automatic merge.

## D4a — Development uses openstreetmap.org's tile server; self-hosting is deferred

**Decided 16 August 2026.** D4 below still describes where this is going. It is not what
development runs against for now.

Downloading the planet archive was attempted and abandoned. The transfer averaged 3.6 MB/s
(about 29 Mbps) from Protomaps' build server, which would have taken roughly ten hours for
127.9 GB, and it silently truncated at 9.2 GB with a successful exit status. Development
therefore points at `tile.openstreetmap.org` directly, and the hosting pipeline is revisited
later.

**Policy clarification, 6 October 2026.** The
[OSM raster tile usage policy](https://operations.osmfoundation.org/policies/tiles/)
permits normal interactive viewing subject to its requirements. It does not categorically
prohibit released or commercial applications. Our development-only use and requirement to
self-host before release are project decisions: service is best-effort, without an SLA,
and access may be withdrawn if usage degrades the service.

Requirements relevant to the current browser implementation:

- Use the documented HTTPS tile URL and keep linked attribution visible on the map.
- Send a valid HTTP Referer; do not suppress it through a restrictive Referrer-Policy.
  Browsers use their normal User-Agent. Non-browser clients must identify their application.
- Honor HTTP caching headers and avoid default no-cache requests. If a cache cannot read
  those headers, retain tiles for at least seven days. Modern browsers with default settings
  satisfy the policy's technical requirements; verify deployed headers and request behavior.
- Do not bulk download, build archives from this server, or offer offline/prefetch features.
  Normal viewport requests with modest, short-range look-ahead are permitted.

**Request settings, 6 October 2026.** `MapCanvas` explicitly uses the browser's default
HTTP cache, restricts credentials to same-origin requests, and uses `strict-origin` as the
request referrer policy. OSM receives the site origin as Referer, without voyage paths or
query strings. The browser supplies its own User-Agent and handles cache validation;
MapLibre keeps expired-tile refresh enabled so it reads the response's expiry headers.
The current app continues using OSM under these conditions; D4 remains the future hosting
direction. See the rule-by-rule audit in `docs/osm-tile-policy.md`.

Official references: [technical requirements](https://operations.osmfoundation.org/policies/tiles/#3-technical-requirements),
[permitted viewing and prohibited bulk/offline use](https://operations.osmfoundation.org/policies/tiles/#4-prohibited-bulk-downloading-scraping-and-offline-use),
[availability and enforcement](https://operations.osmfoundation.org/policies/tiles/#7-changes-and-enforcement),
[attribution guidelines](https://osmfoundation.org/wiki/Licence/Attribution_Guidelines),
and [OSMF Terms of Use](https://osmfoundation.org/wiki/Terms_of_Use).

**The consequence that changes what phase 7 can deliver.** openstreetmap.org serves **raster**
tiles — finished PNG images. The plan assumes **vector** tiles, and the difference is not
cosmetic:

- MapLibre displays raster tiles perfectly well as a raster source, so pins, clustering,
  selection, the mobile photo sheet and the granularity areas all behave identically. None of
  the interaction work is affected.
- But raster tiles **cannot be restyled**. The basemap arrives already drawn in OSM's standard
  cartography, which is bright and high-contrast. Phase 7's target 4 — "styled to match the
  phase 1 design tokens so the map does not look pasted in from another product" — is not
  achievable this way, and neither is the dark basemap in
  `design/components/map-primitives.html`.

So the map will work and will look like OpenStreetMap rather than like PinVoyager until the
tile pipeline is built. Target 4 is deferred with the hosting, not dropped.

## D4 — Map: MapLibre GL JS, OpenStreetMap data, self-hosted Protomaps tiles

Vector tiles served from a `.pmtiles` archive we host ourselves. No third-party tile service
at runtime. No API key in the frontend.

**Why:** no per-load billing as the site grows, no vendor able to break the map, and no key
to leak. The renderer and the data are both open, so the whole stack can be moved.

**Costs accepted:**
- A planet basemap archive is 100+ GB. Regional extracts are far smaller but break for a
  travel app whose users go anywhere — plan for planet.
- `.pmtiles` serving depends on HTTP range requests; the Rust backend must support them.
- OpenStreetMap attribution is required by the ODbL license and must be visible on the map.
- There is no satellite imagery in OSM. A satellite layer, if ever wanted, needs a separate
  provider and is out of scope.

## D5 — No geocoding; place context comes from the basemap

Photos are placed at the coordinates recorded in their EXIF, transformed only by the
granularity setting. **Nothing in the data model associates a photo with a named place.** A
photo row holds a latitude and longitude, or nothing at all.

Place names are still visible to the user — countries, states, cities, towns, districts — but
they come from the basemap, which renders them as map labels. OpenStreetMap data contains all
of those names, and they are baked into the vector tiles we already host. They are *map
context*, not photo data.

**Why:** the names were never the expensive part. The *association* is. Storing a place per
photo means a reverse-geocode call in the upload path, either a metered vendor or a
self-hosted index, plus somewhere to put the result, plus staleness when boundaries change,
plus per-country `admin_level` mapping logic because level 4 means a US state, a Japanese
prefecture, and a German Bundesland. None of that is needed to draw a marker on a map, and a
marker on a map is the whole feature.

**Consequences accepted:**

- Photo metadata in the UI is coordinates plus a precision label, never a place name. The map
  underneath supplies the human-readable context.
- Search-by-place is out. Spatial search is not — a bounding-box or radius query over
  coordinates needs no geocoding at all and is the natural way to do "voyages near here."
- The label language is fixed when the tiles are built, not at runtime. A basemap built from
  OSM's `name` tag shows 京都; one built from `name:en` shows Kyoto. Choose deliberately in
  phase 7, because changing it later means rebuilding the tile archive.
- Which names a viewer sees depends on their zoom level. There is no stable "the place" for a
  photo — which is precisely the point.
- At coarse granularity a marker may sit over a *different* named place than where the photo
  was taken. This is one of the reasons the design draws an area rather than a pin at coarse
  precision: an area over a region claims nothing about the label at its center.

## D6 — Published voyages are public

Draft voyages are owner-only and return 404 to other callers. Once published, a voyage and
its attached photos are public. There is no private or unlisted published mode. Owner-only
photo metadata may include exact coordinates; public responses apply the voyage's chosen
granularity.

**Why:** it is the product. Introducing visibility levels later would touch every query,
every cache, and every authorization check, so it must not be assumed away now.

**Required consequences:**

- **Derived variants must be re-encoded with all metadata stripped.** Published voyages
  make served images public artifacts. If a user selects coarse granularity but
  the served JPEG still carries its original GPS EXIF, the granularity setting is decorative
  and leaks exact coordinates to anyone who downloads the file. Stripping is what makes the
  setting real.
- Original uploads are never served. Only stripped, re-encoded variants are.
- The granularity control must state this guarantee where the choice is made, so the user can
  see what they are getting.

## D7 — Files live on disk but are never served from disk

Uploads are written to a content-addressed path derived from a hash of the content. They are
served exclusively through a Rust handler.

**Why:** a handler can enforce authorization, set a server-determined `Content-Type` rather
than trusting the upload, send `X-Content-Type-Options: nosniff` and a restrictive
`Content-Disposition`, apply rate limits, and log access. Static file serving of a
user-upload directory does none of that, and a stored-XSS via an uploaded file that the
browser sniffs as HTML is the classic outcome.

**Validation requirements:**
- Accept image files only, determined by inspecting magic bytes — never the filename
  extension and never the client-supplied `Content-Type`
- Content-addressed storage paths, never user-supplied filenames — this removes path
  traversal as a category
- Bound decoded pixel dimensions before decoding, to reject decompression bombs
- Enforce per-request and per-user upload size and rate limits

## D8 — All database access is async, and the runtime is never blocked

Every database operation is `async`, on a Tokio multi-threaded runtime, through `sqlx`'s async
API and a `PgPool`. There is no synchronous database path anywhere in the service.

**Why this is nearly free, and what the real work is.** `sqlx` is async-native — it has no
blocking mode to accidentally use — so "all database operations are async" is the default state
of the chosen stack rather than something to engineer. Recording it as a decision is still
worthwhile, because it fixes the inverse obligation, which is *not* free:

> **Nothing that blocks may run on a runtime worker thread.**

An async runtime multiplexes many tasks onto a few OS threads. One task that occupies a thread
without yielding stalls every other task scheduled on it. In a service that is mostly awaiting
Postgres this is invisible — right up until phase 4 starts decoding photographs.

**The specific hazards in this project:**

- **Image decode, resize, and encode are CPU-bound** and are the single biggest risk. A batch
  of fifty 25 MB JPEGs processed inline will starve the runtime and stall unrelated requests.
  This work belongs in `tokio::task::spawn_blocking`, or on a dedicated Rayon pool if the
  parallelism needs shaping.
- **Content hashing** of large uploads is CPU-bound for the same reason.
- **Filesystem access** through `std::fs` blocks. Use `tokio::fs` in request paths — noting
  that it is itself `spawn_blocking` underneath, so for a long sequence of file operations one
  explicit `spawn_blocking` around the whole sequence beats many implicit ones.
- **Holding a pool connection across an unrelated `await`** drains the pool. Acquire late,
  release early, and never hold a connection while an image is being processed.
- **Long-lived transactions across awaits** cause lock contention in Postgres. Keep them short
  and inside a single logical unit of work.

**Async does not remove the connection ceiling.** Task concurrency is effectively unbounded;
pool connections are not. Set `max_connections` deliberately and always set `acquire_timeout` —
without it, pool exhaustion presents as requests hanging indefinitely rather than as a 503,
which is far harder to diagnose under load.

**Blocking bugs are invisible in normal testing.** They surface as tail-latency under
concurrency, not as failures. `tokio-console` in development is the practical way to see a task
occupying a worker thread; the compiler and Clippy will not tell you.

## D9 — HEIC is required, so native image dependencies are accepted

Phase 4 must accept HEIC uploads. iPhones shoot HEIC by default, so declining it would reject
a large share of the photographs this product exists to display.

**This supersedes the original reasoning for the `image` crate.** That choice was recorded as
"pure Rust, no native dependency to manage" — and `image` has no HEIC or HEIF decoder at all.
It is not a feature flag; the decoder does not exist, largely because HEIC wraps HEVC and
carries patent encumbrance the maintainers declined. Supporting HEIC therefore means
`libheif-rs` → `libheif-sys` → the C library `libheif`, and the "no native dependency"
property is gone.

That is an accepted cost, not an oversight. What it obliges:

- **The container needs libheif at both stages.** `libheif-dev` (or equivalent) in the
  builder, and the libheif shared objects in the runtime stage. The Dockerfile currently
  installs only `ca-certificates` and `curl`; a binary linked against libheif will not start
  there. Fix this when phase 4 adds the dependency, not before — an unused native library in
  the image is just attack surface.
- **Every developer machine and CI runner needs it too** (`brew install libheif`,
  `apt install libheif-dev`). Add it to `docs/running.md` prerequisites at the same time.
- **HEVC patent licensing** is a real consideration for a public site at scale. Noted and
  accepted; revisit if the site grows into a commercial product.

**Variants are encoded as lossy WebP via the `webp` crate and libwebp.** The pure-Rust
`image-webp` encoder is lossless-only, and lossless WebP on photographs is typically larger
than the source JPEG — which would invert the reason for choosing WebP at all. libwebp is a
second native dependency, but the marginal cost is small now that libheif is already in, and
both land in the same Dockerfile change.

So the image stack is: `libheif-rs` to decode HEIC, `image` to decode everything else and to
resize, and `webp` to encode the variants. Three pieces rather than one, and the container
needs `libheif` and `libwebp` at both build and runtime.

---

## D10 — A voyage's pins are grouped in the browser; the world's are grouped on the server

**Decided 16 August 2026.** The phase 7 plan said clustering is always server-side. It is not,
and the split is deliberate.

**A voyage is a bounded collection.** `GET /api/voyages/{id}/map` already returns every pin in
it in one response — tens to low hundreds of photographs. The browser groups them as the camera
moves, so a pan or a zoom costs nothing. Doing it server-side would mean adding bounding-box
and zoom parameters to that endpoint and making every gesture a network round trip for data the
browser is already holding. The grouping is a pure function in `web/src/map/geo.ts`, which is
also what makes it directly testable.

**The world is not bounded.** `GET /api/map` answers for every published voyage there is, and
at a world view its rectangle covers every published photograph in the database. Nothing can be
sent whole for the browser to group. So the rectangle and the zoom are the question, and
markers are the answer.

**What this costs.** Two implementations of clustering, in two languages, that will not always
agree: the browser groups greedily by nearest neighbor, the server by a fixed grid. Their
answers differ slightly at the edges of a cell. That is acceptable because they are never shown
side by side — one draws a voyage, the other draws the world — and unifying them would mean
either making a voyage's map chatty or shipping the planet to the browser.

**What the server must never do.** Group first and round afterwards. Every photograph is
rounded to *its own voyage's* granularity before anything is grouped, through the same
`place()` call every other endpoint uses; a marker's published position is one real
photograph's, never a computed center of several. That last part is enforced by the type
system rather than by care: `Coordinate` has private fields and one constructor, so an average
of several coordinates is not a thing that can be serialized. See D6.

---

## D11 — Photos belong to voyages from upload onward

**Decided 6 October 2026.** The first creation step requires a Voyage Name and
Voyage Description and tells the user that photo uploads follow at the next step.
Continuing creates a private draft with those details before showing the upload
step. Dropping or choosing photos at that step starts uploading automatically.
Adding photos to an existing voyage opens the upload step directly. Each accepted
photo belongs to its voyage immediately; no photo bytes are sent without a voyage.
There is no standalone photo library or later library-to-voyage attachment
endpoint. Photos without GPS stay in the voyage.
Uploads append in submission order; owners can change the order in the editor.

Each photo record has exactly one voyage, enforced by a non-null foreign key with
matching ownership. Removing a photo deletes its record; deleting a voyage deletes
its photo records. Content-addressed image files are removed only when no remaining
photo references their hash. Existing unattached photos migrate into owner-only
“Recovered photos” drafts. Existing photos shared by multiple voyages become separate
photo records referencing the same stored bytes, preserving each voyage's contents.

Draft owners have prominent Publish voyage buttons on the voyage page at both
screen widths and in the edit page header. Publishing remains a deliberate action;
creating a voyage or uploading photos never publishes it. D6 visibility still applies.
This supersedes the earlier library and deletion behavior in phases 4–6.

## D12 — Voyage markers open a location photo gallery

**Decided 6 October 2026.** On the Voyage page, activating a pin or a numbered
cluster opens a native modal HTML dialog containing that marker's photos, in
voyage order. “Around this location” means membership in the existing cluster
at the current zoom (D10); it introduces no new distance threshold or exact
coordinate lookup. A single pin shows its one photo. Cluster activation opens
the gallery instead of zooming; zoom controls still expand groups. Discovery
navigation is unchanged. Existing photo-list selection remains available.

The gallery uses only the voyage detail payload and authorized, metadata-free
variants (D6/D7). Pending and failed photos have explicit states; readiness
polling updates an open gallery. Use native `dialog.showModal()`, Escape
cancellation, and a `form method="dialog"` close action instead of recreating
modal focus trapping or background inertness in JavaScript.

**Updated 7 October 2026.** The dialog shows one photo at a time in a viewport-
bounded frame without a gallery scrollbar. Native previous/next buttons and
Left/Right arrow keys browse the existing order, stopping at its boundaries.
An icon-only X closes it from the upper-right corner. `closedby="any"` enables
native backdrop dismissal; a pointer-based fallback provides the same behavior
where that attribute is unsupported. Starting a drag inside the dialog and
releasing outside must not dismiss it. The active photo is retained by ID while
readiness updates arrive; if it disappears, the gallery shows the first remaining
photo. Native Escape and dialog close-form behavior remain.


## Defaults that can change

Chosen as reasonable starting points. Change them here if you disagree; nothing depends on
them deeply yet.

| Area | Default | Reasoning |
|---|---|---|
| Async runtime | Tokio, multi-threaded | The ecosystem default; axum and sqlx both build on it (D8) |
| Web framework | axum | Tower middleware ecosystem, tokio-native, the mainstream idiomatic choice |
| Database | PostgreSQL via sqlx | Async-native, real constraints, concurrent writes during batch uploads, compile-time checked queries |
| Frontend build | Vite + React + TypeScript | Fast, conventional, no framework lock-in |
| Sessions | Server-side, opaque cookie ID | Revocable on demand. A stateless JWT session cannot be revoked, which matters more here because the account *is* the identity |
| Image processing | `image` crate **plus `libheif-rs`** | See D9 — HEIC is required, and `image` cannot decode it |
