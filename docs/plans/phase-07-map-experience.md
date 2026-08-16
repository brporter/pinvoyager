# Phase 7 — Map experience

**Status:** not started
**Depends on:** phase 6
**Owner:** Claude

## Goal

Voyage detail shows a real interactive map. Tiles are served from our own `.pmtiles` archive
with no third-party runtime dependency and no API key in the frontend (D4). Photos appear as
pins, dense areas cluster, and selection moves in both directions between the map and the
photo list.

## Non-goals

- No route lines or path drawing between pins. Voyages are collections of places, not tracks —
  unless that changes, in which case say so here first.
- No place search or geocoding (D5).
- No satellite basemap. OSM has no aerial imagery (D4).
- No location editing.

## Targets

1. Acquire an OpenStreetMap-derived planet `.pmtiles` basemap and document the acquisition and
   refresh procedure — this is operational knowledge that is painful to reconstruct later.
2. Tile serving. Two architectures are viable, and Rust supports both well:

   **(a) Serve the archive, let the client read it.** `pmtiles.js` inside MapLibre issues HTTP
   Range requests against the `.pmtiles` file and decodes the format in the browser. The
   server only has to honour ranges, which `tower-http`'s file serving does. Least code.

   **(b) Decode server-side and serve `/{z}/{x}/{y}.mvt`.** The `pmtiles` crate reads the
   format natively, so this is a genuinely small amount of work in Rust. Smaller client
   payloads, fewer round trips, and far better CDN cache behaviour — per-tile URLs cache
   cleanly where byte ranges into one 100 GB file do not.

   Given the `pmtiles` crate exists, (b) is the better default here. **HTTP range request
   support is mandatory** under either.
3. Tile responses carry correct caching headers, and tile requests are cheap enough to serve
   under the load a map pan generates — one gesture can request dozens of tiles.
4. MapLibre GL JS in voyage detail, styled to match the phase 1 design tokens so the map does
   not look pasted in from another product.
5. OpenStreetMap attribution visible and correctly worded. This is an ODbL license
   requirement, not a courtesy.
6. Pins from the phase 5 map endpoint, with the design's pin, selected-pin, and cluster
   treatments.
7. Clustering with count tiers, expanding on zoom.
8. **Bidirectional selection** — clicking a pin highlights and scrolls to its photo; selecting
   a photo pans the map and marks its pin. This is the interaction the whole screen exists for.
9. Mobile treatment: full-bleed map with a draggable photo sheet, per the phase 1 design.
   Gestures must not fight — the sheet drag and the map pan cannot both claim a vertical swipe.
10. Granularity is honored visually. A coarse voyage shows an area indication rather than a
    pin implying precision the data does not have. A pin drawn at the center of a 10 km
    rounding reads as an exact claim, and that is misleading.
11. Unplaced photos have a visible home in the layout, per phase 1.
12. Map initialization is lazy and does not block first paint — MapLibre plus a style is a
    substantial bundle.

## Acceptance criteria

- **T2** — `curl` with a `Range` header returns 206 and the correct byte slice. Without range
  support the map will appear to work locally and fail badly under real use.
- **T3** — Panning across a city does not produce visible tile stalls on a local build.
- **T5** — Attribution is present on every map instance.
- **T8** — Both directions verified by hand, at both breakpoints.
- **T9** — On a real touch device, dragging the sheet does not pan the map and vice versa.
- **T10** — A voyage at ~10 km granularity renders an area, not a point.
- **T12** — Lighthouse performance does not regress materially against the phase 6 baseline
  on voyage detail.

## Design references

- `design/screens/voyage-detail.html`
- `design/components/map-primitives.html`
- `design/components/granularity.html`

## Open questions

- **Tile hosting operationally.** A planet archive is 100+ GB. Where does it live in
  development — is every developer expected to download it, or is there a shared instance and
  a small regional extract for local work?
- **Basemap refresh cadence.** OSM data ages. Quarterly is probably fine for this use, but it
  needs an owner and a documented procedure.
- **The basemap's label language is fixed at tile-build time.** Building from OSM's `name` tag
  yields local-language labels (京都); building from `name:en` yields English (Kyoto). Per D5
  these labels are the *only* place names anywhere in the product, so the choice carries more
  weight here than it normally would — and changing it means regenerating the archive.
- Whether clustering runs client-side in MapLibre or server-side in phase 5's endpoint.
  Client-side is far simpler and fine until a voyage has thousands of photos.
- Whether the discover screen also gets a map.

## Notes

T10 is a judgment call worth taking seriously. Granularity's purpose is that users can be
imprecise on purpose; drawing a sharp pin at a rounded coordinate visually undoes the choice
they made. An area circle whose radius reflects the actual precision tells the truth.

Self-hosting tiles is the phase's main operational cost and was accepted deliberately (D4) to
avoid per-load billing, API keys in the client, and a vendor able to break the map. The
tradeoff is real: disk, bandwidth, and a refresh procedure are now ours.
