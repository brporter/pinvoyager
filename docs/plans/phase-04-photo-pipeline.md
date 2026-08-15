# Phase 4 — Photo pipeline

**Status:** not started
**Depends on:** phase 3
**Owner:** Claude

## Goal

A signed-in user can upload image files; each is validated as a real image, stored on disk
content-addressed, has its EXIF capture time and GPS coordinates extracted, and has derived
variants generated that carry **no metadata whatsoever**. Images are served only through an
authorizing handler, never from disk directly, per D7.

## Non-goals

- No voyage association (phase 5). Photos land in the uploader's library.
- No editing of location (explicitly a future feature).
- No geocoding or place names (D5).
- No video, no RAW, no HEIC unless it falls out for free — decide deliberately, not by
  accident.

## Targets

1. `POST /api/photos` — multipart upload, one or many files, streaming to disk rather than
   buffering whole files in memory.
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
6. Variant generation runs off the request path so a large batch does not hold a connection
   open. Photos surface in a pending state until their variants exist.
7. `GET /api/photos/{id}/{variant}` — serves from disk through the handler with a
   server-determined `Content-Type`, `X-Content-Type-Options: nosniff`, a restrictive
   `Content-Disposition`, `ETag`/`Last-Modified`, and long-lived immutable caching (safe
   because the ID is content-derived).
8. `GET /api/photos/{id}` — metadata: dimensions, capture time, coordinates at the granularity
   in force, and whether the photo is placed.
9. `DELETE /api/photos/{id}` — owner only. Removes variants; removes the original only when no
   other photo row references the same hash.
10. Per-user upload rate limiting and a per-request size cap, both returning honest status
    codes (429, 413) rather than a connection reset.

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
- **T7** — Response headers are as specified. Requesting a variant that does not exist returns
  404, not a directory listing or a path-traversal opportunity. Confirm `../` in any path
  position cannot escape the storage root.
- **T9** — Deleting one of two photos sharing a hash leaves the other's image serving fine.

## Design references

- `design/screens/upload-flow.html`
- `design/components/upload.html`
- `design/flows/upload.html` — including the no-GPS branch

## Open questions

- **Granularity application point.** Store exact coordinates and round at serve time (allows
  later granularity changes and the future "adjust location" feature, but exact coordinates
  sit in the database), or round at ingest and discard the original (stronger privacy, but the
  precision is gone forever and cannot be recovered if the user later wants finer granularity).
  **This needs a decision before building T8** — it is not reversible after data exists.
- Which variant sizes, and whether to emit WebP/AVIF alongside JPEG.
- HEIC support. iPhones shoot HEIC by default, so declining it is a real product limitation,
  but decoding it in pure Rust is not straightforward.

## Notes

Serving through a handler is what allows every one of those response headers to be set
correctly. The failure mode being avoided is a stored XSS: a user uploads a file the browser
sniffs as HTML, and it executes on our origin against another user's session.

Deduplication by hash and public voyages interact: two users uploading the same bytes share
one file. That is fine for storage, but deletion must be reference-counted (T9) or one user
deleting removes another user's photo.
