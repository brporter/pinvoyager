# Phase 7 — Map experience

**Status:** complete, except for targets 1–3 (deferred under D4a) and the hand checks below
**Depends on:** phase 6
**Owner:** Claude

## Goal

Voyage detail shows a real interactive map. Tiles are served from our own `.pmtiles` archive
with no third-party runtime dependency and no API key in the frontend (D4). Photos appear as
pins, dense areas cluster, and selection moves in both directions between the map and the
photo list. Every developer will download the entire tile map for now. OSM basemap updates
will be handled later. Basemap label language will be fixed at English for now. Clustering
is always server-side. Discover screen also has a map.

## Non-goals

- No route lines or path drawing between pins. Voyages are collections of places, not tracks —
  unless that changes, in which case say so here first.
- No place search or geocoding (D5).
- No satellite basemap. OSM has no aerial imagery (D4).
- No location editing.

## Targets

**Targets 1 and 2 are deferred, and target 4 is partly blocked by that — see D4a.**
Development points at `tile.openstreetmap.org` instead. The interaction work (targets 5 to 12)
is unaffected and is what this phase now delivers.

1. ~~Acquire an OpenStreetMap-derived planet `.pmtiles` basemap~~ — **deferred.** The download
   was attempted: 3.6 MB/s from Protomaps' build server, which is about ten hours for 127.9 GB,
   and it truncated at 9.2 GB while reporting success. Revisit with a different source, a
   regional extract, or a locally built archive.
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
4. MapLibre GL JS in voyage detail. **The styling half of this target is blocked by D4a.**
   Raster tiles arrive already drawn, so the basemap cannot be recoloured to match the design
   tokens and the dark basemap in `design/components/map-primitives.html` is unreachable until
   vector tiles are in place. Build the map so that swapping the source later is a change to
   one style definition rather than to the map code.
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

## Notes

T10 is a judgment call worth taking seriously. Granularity's purpose is that users can be
imprecise on purpose; drawing a sharp pin at a rounded coordinate visually undoes the choice
they made. An area circle whose radius reflects the actual precision tells the truth.

Self-hosting tiles is the phase's main operational cost and was accepted deliberately (D4) to
avoid per-load billing, API keys in the client, and a vendor able to break the map. The
tradeoff is real: disk, bandwidth, and a refresh procedure are now ours.

## What was built

Everything lives under `web/src/map/`, and the division of it is the point:

| file               | what it owns                                                     |
|--------------------|------------------------------------------------------------------|
| `style.ts`         | the tile source. The only file that names a tile URL.            |
| `geo.ts`           | projection, clustering, area sizing, camera fitting. No MapLibre.|
| `MapCanvas.tsx`    | the MapLibre instance, camera, zoom controls, attribution.       |
| `VoyageMap.tsx`    | one voyage: pins, clusters, granularity areas, selection.        |
| `DiscoverMap.tsx`  | every voyage: markers grouped by the server.                     |
| `Lazy*.tsx`        | the two code-splitting boundaries.                               |

`MapCanvas.tsx` is the only file in the application that imports `maplibre-gl`. Swapping the
basemap for the self-hosted vector archive means rewriting `style.ts`; swapping MapLibre for
another library means rewriting `MapCanvas.tsx`. Neither reaches the pin behaviour.

Pins and clusters are real DOM buttons positioned by MapLibre markers, not features drawn into
the WebGL canvas. A pin drawn on a canvas cannot be tabbed to, has no accessible name, and
cannot carry the 44 px hit area the design requires — and the pin, cluster and area treatments
already exist as CSS in `design/components.css`, so drawing them again as MapLibre paint
properties would make the design system a description of the map rather than its source.

## Deviations, and why

**Clustering runs in the browser for a voyage and on the server for the world.** The goal above
says clustering is always server-side. It is not, and the split is now recorded as decision
D10. A voyage arrives whole in one response, so grouping it in the browser makes a pan cost
nothing; the world cannot arrive whole, so `GET /api/map` takes a rectangle and a zoom and
answers with markers.

**The voyage detail map reads the detail payload, not `GET /api/voyages/{id}/map`.** Target 6
names the map endpoint. The map is only rendered once the detail read has resolved — both the
desktop split and the mobile sheet need the photo list to exist — so asking for the map payload
as well would be a second round trip for coordinates already in hand. The detail payload
carries the same rounded `Coordinate`, the same `precision_m`, and, unlike the map payload, the
capture times the pin labels are made of. `GET /api/voyages/{id}/map` therefore still has no
consumer in the frontend; the discovery map reads `GET /api/map` instead, which is a different
endpoint answering a different question.

**A coarse voyage draws a circle *and* a pin, where the design draws the circle alone.**
`design/components/map-primitives.html` shows a rounded position as a dashed circle with no
marker in it. That treatment cannot be selected: `.pv-map-area` sets `pointer-events: none`, it
is not a button, and it is not in the tab order — so a coarse voyage would have no way to reach
a photograph from the map, and bidirectional selection is the interaction the screen exists
for. A pin ringed by a dashed 20 km circle is the ordinary cartographic convention for a
position with an uncertainty and does not read as an exact claim. **The design file should be
updated to settle this.**

## Verification

- **T5** — asserted in `src/map/__tests__/VoyageMap.test.tsx`: the notice is present on the map
  and links to `https://www.openstreetmap.org/copyright`.
- **T10** — asserted in the same file: a voyage with `precision_m` draws `.pv-map-area`
  elements, a voyage at exact precision draws none, and the camera stops short of the zoom at
  which the circle would be smaller than the rounding.
- **T12** — measured on a production build. The entry chunk is 356.22 kB (108.72 kB gzipped)
  against phase 6's 354.95 kB; MapLibre is a separate 950.96 kB chunk (248.36 kB gzipped) with
  its own 82.86 kB stylesheet, shared by both map boundaries so moving from discovery to a
  voyage does not download it twice. Searching the built entry chunk for the string `maplibre`
  returns zero matches.
- The discovery map's own behaviour is asserted in `src/map/__tests__/DiscoverMap.test.tsx`, and
  the endpoint's in `crates/api/tests/discover_map.rs`, which drives the production router.
- The arithmetic in `geo.ts` is checked against independently derived figures — 156543.03 m/px
  at the equator at zoom 0, the `cos(latitude)` correction, and `zoomForPrecision` being the
  exact inverse of `areaDiameterPx`.
- The tests were shown not to be vacuous by breaking the implementation and confirming each
  break failed a test. In the browser: removing the latitude correction from `metresPerPixel`;
  letting an absorbed pin absorb its own neighbours; drawing the granularity circle at the
  precision radius instead of its diameter; panning the camera for a pin click as well as a list
  click; leaving the selected pin in the clustering pool; and ignoring `prefers-reduced-motion`
  in camera movement. On the server: dropping `v.published` from the rectangle query; rounding
  every voyage as exact; treating a rectangle that crosses the antimeridian as an ordinary
  range; and returning rows in an unspecified order.
- One deliberate break did **not** fail anything, and the comment claiming it would was wrong.
  Removing the explicit `is_finite` guard from the bounding-box parser left NaN still refused,
  because `RangeInclusive::contains` compares with `PartialOrd` and every comparison against NaN
  is false. The guard was kept and its comment corrected: it earns its place against the *other*
  obvious spelling of the range test, `south < -90.0 || south > 90.0`, where both halves are
  false for NaN and it would sail through.

## A bug the tests found

Selecting a photograph in the list panned the camera; the camera settling fired MapLibre's
`moveend`; that told `MapCanvas` where it was now looking, which was a state change; the state
change rebuilt the `moveTo` callback; and the effect that pans to a selection had `moveTo` in
its dependency list, so it ran again and panned again. An endless flight, in a browser as much
as in a test — the frontend suite went from 1.4 seconds to never finishing.

`moveTo` is now built with `useCallback` keyed on the map instance alone, so its identity
survives camera movement, and the visible rectangle is compared by value before being stored so
a camera movement that ends where it began does not re-render every marker. `VoyageMap.test.tsx`
asserts that one selection produces exactly one camera movement.

## The discovery map and its endpoint

`GET /api/map?west=&south=&east=&north=&zoom=` returns markers for every published voyage
inside a rectangle. No session is taken and none is read: everything it draws is published, so
the answer is the same for everyone.

Three properties are worth stating because they are what the tests are about.

**Rounded before grouped.** Each photograph passes through its own voyage's granularity — the
same `place()` call every other endpoint uses — and only then is grouped. Grouping exact
coordinates and publishing the group's position would make what the map draws a function of
values that were never allowed out.

**A marker's position is a real photograph's published position**, not a computed centre. That
is forced rather than chosen: `Coordinate` has private fields and one constructor, so an
average of several coordinates is not something that can be serialized at all. The
representative is the lowest photograph id in the group, so the same rectangle draws the same
markers twice running.

**Truncation is reported.** One request considers at most 20,000 photographs; beyond that
`truncated` is true and the map says so. A map that silently dropped half a continent looks
exactly like a map of a continent with fewer photographs on it.

`photos_location_idx` was added for the rectangle query — partial on `latitude IS NOT NULL`,
leading on latitude. It is a btree, not a spatial index, so a wide short rectangle reads more of
the index than it returns; PostGIS would fix that and is a large thing to require of every
deployment for one endpoint. Measure before assuming it is the problem.

**Where the map goes on the discovery screen is a visual decision the design has not made.**
`design/screens/discover.html` draws no map. It is placed between the hero and the voyage grid,
as a band rather than a full screen, using only components the design system already defines.
The design file should settle it.

## Still outstanding

- **T2, T3** — deferred with targets 1 and 2 under D4a. No `.pmtiles` archive exists to range-
  request.
- **T8 at both breakpoints by hand, and T9 on a real touch device.** The gestures cannot fight
  in the way the design warns about, because the sheet is a separate element above the map and
  `.pv-sheet` carries `touch-action: pan-y` — but that reasoning is not a substitute for
  dragging the sheet on a phone.
- **T12 against Lighthouse.** The chunk measurement above is evidence the map is not in the
  entry bundle; it is not a Lighthouse score.
- **Nothing has been run against a real tile server.** Every test here replaces MapLibre with a
  substitute, because jsdom has no WebGL context. That the map draws the right markers is
  checked; that it draws them over a basemap has only been reasoned about.
- **The discovery map opens at a world view**, so its first request asks about the whole planet
  and will hit the 20,000-photograph cap once there is enough published. What it should open on
  instead — the reader's last position, or the densest region — is an open product question.
