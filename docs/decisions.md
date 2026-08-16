# Decisions

Decisions that are settled. Later phases build on these rather than reopening them.

If you want to change one, edit it here first — this file outranks any plan document, and a
phase plan that contradicts it is wrong.

Each entry records the decision and *why*, because the reasoning is what tells you whether a
future change invalidates it.

---

## D1 — Authentication is delegated; no credentials are stored

OIDC/OAuth2 against the four providers, implemented with ASP.NET Core's external
authentication handlers. The application stores
**no credentials of any kind**: no passwords, no password hashes, no long-lived provider
access or refresh tokens.

The provider's ID token is verified on receipt and then discarded. What persists is only the
fact of the identity — which provider, and that provider's stable subject identifier.

**Why:** credentials that are never stored cannot leak. This removes password reset flows,
hashing parameter choices, credential-stuffing exposure, and the breach-notification surface
that comes with holding secrets.

**Explicitly not ASP.NET Core Identity.** Identity is the framework's local-credential system —
password hashes, lockout state, two-factor secrets, recovery codes. Using it here would create
a schema full of credential columns that must remain permanently empty, which is worse than
omitting them: it invites someone later to start filling them. External authentication plus
cookie authentication is the whole of it.

**What "verify" means here** — the framework's handlers perform these checks, so the risk
shifts from implementing them to *configuring them correctly*. Knowing what they do on your
behalf is what lets you tell a genuine failure from a misconfiguration:

- Fetch and cache each provider's JWKS; validate the ID token's signature against it
- Validate `iss`, `aud`, and `exp`
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

## D4 — Map: MapLibre GL JS, OpenStreetMap data, self-hosted Protomaps tiles

Vector tiles served from a `.pmtiles` archive we host ourselves. No third-party tile service
at runtime. No API key in the frontend.

**Why:** no per-load billing as the site grows, no vendor able to break the map, and no key
to leak. The renderer and the data are both open, so the whole stack can be moved.

**Costs accepted:**
- A planet basemap archive is 100+ GB. Regional extracts are far smaller but break for a
  travel app whose users go anywhere — plan for planet.
- `.pmtiles` serving depends on HTTP range requests; the backend must support them. ASP.NET
  Core does natively, and serving the archive for the client to range-read avoids needing a
  PMTiles reader in C# at all.
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
  precision: an area over a region claims nothing about the label at its centre.

## D6 — All voyages are public

There is no private or unlisted mode. Anything uploaded and attached to a voyage is world-
readable.

**Why:** it is the product. Introducing visibility levels later would touch every query,
every cache, and every authorization check, so it must not be assumed away now.

**Consequences that are load-bearing, not optional:**

- **Derived variants must be re-encoded with all metadata stripped.** Every voyage being
  public means a served image is a public artifact. If a user selects coarse granularity but
  the served JPEG still carries its original GPS EXIF, the granularity setting is decorative
  and leaks exact coordinates to anyone who downloads the file. Stripping is what makes the
  setting real.
- Original uploads are never served. Only stripped, re-encoded variants are.
- The granularity control must state this guarantee where the choice is made, so the user can
  see what they are getting.

## D7 — Files live on disk but are never served from disk

Uploads are written to a content-addressed path derived from a hash of the content. They are
served exclusively through an application endpoint.

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

---

## Defaults, not yet load-bearing

Chosen as reasonable starting points. Change them here if you disagree; nothing depends on
them deeply yet.

| Area | Default | Reasoning |
|---|---|---|
| Runtime | .NET 10 (LTS) | Current long-term-support release |
| Web framework | ASP.NET Core Minimal APIs | Small endpoint surface; controller ceremony buys nothing at this size |
| Database | PostgreSQL via EF Core + Npgsql | Real constraints, concurrent writes during batch uploads, and migrations in the box |
| Frontend build | Vite + React + TypeScript | Fast, conventional, no framework lock-in |
| Sessions | Cookie auth backed by a custom `ITicketStore` | Revocable on demand. ASP.NET's default self-contained encrypted ticket cannot be revoked before expiry, which matters more here because the account *is* the identity |
| Data Protection | Key ring persisted to PostgreSQL | Without this, sessions break on restart and cannot be shared across instances |
| Image processing | Undecided — see phase 4 | ImageSharp, SkiaSharp, and Magick.NET differ on licensing, native dependencies, and HEIC support. `System.Drawing.Common` is out: Windows-only since .NET 6 |
| Testing | xUnit + `WebApplicationFactory` + Testcontainers | Integration tests run against a real ephemeral PostgreSQL with no manual setup |
