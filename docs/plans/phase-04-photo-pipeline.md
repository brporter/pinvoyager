# Phase 4 — Photo pipeline

**Status:** not started
**Depends on:** phase 3
**Owner:** Claude

## Goal

A signed-in user can upload image files; each is validated as a real image, stored on disk
content-addressed, has its EXIF capture time and GPS coordinates extracted, and has derived
variants generated that carry **no metadata whatsoever**. Images are served only through an
authorizing handler, never from disk directly, per D7. Store exact coordinates and round at 
serve time (allows later granularity changes and the future "adjust location" feature, but 
exact coordinates sit in the database). HEIC support is a must, and photos stored should be
stored in WebP.

## Non-goals

- No voyage association (phase 5). Photos land in the uploader's library.
- No editing of location (explicitly a future feature).
- No geocoding or place names (D5).
- No video, no RAW, no HEIC unless it falls out for free — decide deliberately, not by
  accident.

## Targets

1. `POST /api/photos` — multipart upload, one or many files, streaming to disk rather than
   buffering whole files in memory. Async throughout: the multipart stream is consumed with
   `await`, and writes go through `tokio::fs` rather than `std::fs` (D8).
2. Validation, in this order, rejecting early: request size cap → magic-byte sniff to confirm
   an image type → decode header to read dimensions → reject beyond a maximum pixel count.
   The filename extension and client `Content-Type` are never trusted for anything.
3. Content-addressed storage: `data/originals/{aa}/{bb}/{full-hash}`, sharded to keep
   directory sizes sane. Uploading the same bytes twice stores one copy.
4. EXIF extraction: capture timestamp, GPS latitude/longitude, orientation. Absent GPS is a
   normal, expected outcome — not an error.
5. Derived variants (thumbnail, display, and full-view sizes) re-encoded from decoded pixels
   with **all metadata stripped** and EXIF orientation baked into the pixels. Per D6 this is a
   correctness requirement: it is what makes the granularity setting real rather than
   cosmetic.
6. **All image work runs under `spawn_blocking`** (D8). Decode, resize, and encode are
   CPU-bound and will stall runtime worker threads if run inline — a fifty-file batch of large
   JPEGs is enough to make unrelated requests time out. Content hashing gets the same
   treatment for the same reason.
7. Variant generation runs off the request path via a worker task fed by a bounded channel, so
   a large batch neither holds a request open nor lets an unbounded queue grow. Photos surface
   in a pending state until their variants exist.
8. **No pool connection is held across image processing.** Read what is needed, release the
   connection, do the CPU work, reacquire to write the result. Holding one for the duration of
   a batch drains the pool and stalls every other request in the service.
9. `GET /api/photos/{id}/{variant}` — serves from disk through the handler with a
   server-determined `Content-Type`, `X-Content-Type-Options: nosniff`, a restrictive
   `Content-Disposition`, `ETag`/`Last-Modified`, and long-lived immutable caching (safe
   because the ID is content-derived). Streamed asynchronously, not read into memory first.
10. `GET /api/photos/{id}` — metadata: dimensions, capture time, coordinates at the granularity
    in force, and whether the photo is placed.
11. `DELETE /api/photos/{id}` — owner only. Removes variants; removes the original only when no
    other photo row references the same hash.
12. Per-user upload rate limiting and a per-request size cap, both returning honest status
    codes (429, 413) rather than a connection reset.
    **Two things phase 2 left for this phase to resolve deliberately:** there is no config
    field for the maximum request body — phase 2 hardcoded a 32 MiB global ceiling — and a
    body limit applied as an outer layer around the whole router can only ever be *tightened*
    by inner layers, never relaxed. If the upload route needs a larger body than the global
    ceiling, the limit has to move off the global stack and onto individual routes. Decide
    this before writing the handler, not after discovering 413s on valid uploads.

## Acceptance criteria

- **T2** — A `.jpg` file whose bytes are actually HTML is rejected. A 50,000 × 50,000 PNG
  decompression bomb is rejected on dimensions before it is ever fully decoded. A file above
  the size cap returns 413.
- **T3** — Uploading identical bytes twice produces two photo rows and one file on disk.
- **T4** — A photo with GPS EXIF yields coordinates; a screenshot with none is accepted and
  marked unplaced.
- **T5** — **Run `exiftool` against every generated variant: it reports no GPS, no timestamp,
  no camera make or model, no metadata at all.** A rotated photo displays upright with no
  `orientation` tag present. This is the single most important check in the phase.
- **T6/T8** — **Upload a batch of fifty large photos and hammer an unrelated endpoint
  concurrently.** Its latency must stay flat. If it degrades, image work is running on runtime
  worker threads or a pool connection is being held across processing — this is the check that
  proves D8 is actually being honoured, and it is invisible to any single-request test.
- **T7** — Filling the bounded channel applies backpressure rather than growing without limit
  or dropping work silently.
- **T9** — Response headers are as specified. Requesting a variant that does not exist returns
  404, not a directory listing or a path-traversal opportunity. Confirm `../` in any path
  position cannot escape the storage root.
- **T11** — Deleting one of two photos sharing a hash leaves the other's image serving fine.

## Design references

- `design/screens/upload-flow.html`
- `design/components/upload.html`
- `design/flows/upload.html` — including the no-GPS branch

## Notes

**Verify the strip, do not assume it.** The `exiftool` acceptance criterion exists because
"we re-encoded it" is not the same as "we stripped it" — encoders differ in what they carry
through a decode/encode round trip, and a dependency upgrade can change that default underneath
you. This deserves an automated test asserting the absence of GPS tags on a generated variant,
not a one-time manual check.

Serving through a handler is what allows every one of those response headers to be set
correctly. The failure mode being avoided is a stored XSS: a user uploads a file the browser
sniffs as HTML, and it executes on our origin against another user's session.

Deduplication by hash and public voyages interact: two users uploading the same bytes share
one file. That is fine for storage, but deletion must be reference-counted (T9) or one user
deleting removes another user's photo.
