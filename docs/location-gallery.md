# Location marker photo gallery

Decision D12 in `docs/decisions.md` adds a gallery to Voyage page markers.

## Behavior

Activating a single pin opens its photo. Activating a numbered cluster opens
all photos represented by that cluster at the current zoom, in voyage order.
“Photos at this location” describes that existing grouping, not a new distance
search. The detail payload already applies public coordinate precision, so the
gallery never fetches owner-only exact coordinates or introduces a new endpoint.
Photos without coordinates are not members of map markers and remain in the
voyage's unplaced section. Discovery retains its existing navigation behavior.

`web/src/map/VoyageMap.tsx` (`VoyageMapProps.onOpenGallery`, `VoyageMarkers`,
`ClusterMarker`) sends marker member IDs to the page. On the Voyage page, opening
a cluster replaces its previous click-to-zoom action; the zoom controls continue
to split groups. Other map consumers without the gallery callback retain their
existing cluster expansion. Single-pin selection still highlights and scrolls
the corresponding photo. Selecting a photo in the rail continues to pan its pin.

`web/src/screens/VoyageDetail.tsx` (`VoyageDetail`) stores the selected IDs and
filters the current query payload in its existing photo order. This works with
both desktop and mobile layouts. It preserves the IDs while image processing
updates the payload, so an open gallery receives newly ready images through
`useVoyage` polling. If photos disappear from a refreshed payload, they disappear
from the gallery too; the close action remains available.

## HTML and visual treatment

`web/src/components/LocationGallery.tsx` (`LocationGallery`) uses:

- A native `<dialog>` opened by `showModal()` with a labeled heading.
- Native modal focus containment, background inertness, and Escape cancellation.
- An actual `<form method="dialog">` with an icon-only X submit button in the
  upper-right corner, named “Close gallery” for assistive technology.
- Native previous/next buttons with arrow icons, descriptive names, and disabled
  states at the first and last photo. Left/Right arrow keys perform the same actions.
- One `<figure>` / `<figcaption>` at a time, with descriptive image alt text and
  a live “1 of N” counter. Selection is tracked by photo ID so readiness and order
  changes do not replace the active photo; removal falls back to the first photo.
- Authorized display WebP variants, explicit image dimensions, and asynchronous
  decoding. Only the active image is rendered and requested; originals are never
  requested.
- `closedby="any"` for native backdrop dismissal, with a primary-pointer fallback
  for browsers that do not implement it. The pointer press and click must both
  occur outside the dialog bounds; content, padding and inside-to-outside drags
  do not dismiss the dialog.
- Explicit processing and failed-processing messages instead of hidden photos.

The close event clears gallery state. Native focus restoration is retained; the
page additionally restores the invoking marker or its equivalent selected pin
when React replaced that button during selection. If the marker no longer exists,
the close action succeeds without attempting to focus a missing element. Escape
uses the browser's native cancellation behavior; no custom keyboard trap is added.

The implementation follows the [HTML Standard dialog element](https://html.spec.whatwg.org/multipage/interactive-elements.html#the-dialog-element)
and [dialog close behavior](https://html.spec.whatwg.org/multipage/interactive-elements.html#dom-dialog-close),
including [light dismissal](https://html.spec.whatwg.org/multipage/interactive-elements.html#dialog-light-dismiss).

This is a deliberate extension of `design/screens/voyage-detail.html` rather than
an existing reference screen. `web/src/styles/index.css` (`pv-location-gallery`)
uses shared colors, spacing, radii, elevation, and focus styling. The modal is
bounded by the viewport and shows one image contained within the available area,
with navigation to either side. Its layout uses a fixed header/footer and a
flexible image frame with `min-height: 0` and `object-fit: contain`. The dialog
uses `overflow: hidden`, so gallery navigation does not require a scrollbar.
The same layout fits narrow screens. The backdrop uses `--pv-overlay`.

## Verification

Tests were added before implementation. Four new map/page cases failed for the
missing marker callback and dialog while 53 existing cases passed. The component
suite initially failed because `LocationGallery` did not exist.

- `web/src/map/__tests__/VoyageMap.test.tsx`, `VoyageMap — location gallery`:
  cluster membership, no camera zoom, ordinary pins and selected pins.
- `web/src/screens/__tests__/VoyageDetail.test.tsx`, `VoyageDetail — location gallery`:
  desktop/mobile, membership and order, close action, focus restoration after
  single-pin replacement, and refreshed removal of the marker's photo.
- `web/src/components/__tests__/LocationGallery.test.tsx`: native dialog structure,
  dialog close form, labels, image URLs, pending/failed states, readiness updates,
  uncanceled Escape event, unmount cleanup, StrictMode effect replay, and queued
  cleanup close events.
- `web/src/screens/__tests__/VoyageUploadFlow.test.tsx`: real API hooks and HTTP
  transport substitutes verify that pending images become visible in an open
  gallery as three photos finish separately, without a reload.

`web/src/test/setup.ts` supplies the native dialog methods and form close default
action that jsdom lacks. This verifies application wiring; it does not verify
browser top-layer rendering, native focus trapping, real image decoding, or
Escape's default action. Localhost browser automation remains blocked by the
saved browser preference. Those browser checks, including mobile layout and
keyboard-only opening/closing, remain unverified.

Final checks on 6 October 2026:

- `npm --prefix web run test -- --coverage --coverage.reportsDirectory=/tmp/pinvoyager-gallery-coverage`:
  all 466 tests pass across 41 files, with 100% statements, branches, functions,
  and lines; no coverage exclusions were added.
- Type checking, web lint, production build, and `git diff --check` pass. Existing
  Fast Refresh/config-loader and bundle-size warnings remain.
- The StrictMode regression failed before its cleanup guard was added, then
  passed. An open dialog now ignores internal or queued cleanup close events.
- API readiness and the running Vite server respond successfully. Browser visual
  checks and native focus-trap/Escape-default verification remain unperformed.
- Backend code and API contracts did not change, so backend tests were not rerun.
  The last measured backend coverage remains 98.11% lines, 96.35% functions, and
  95.05% regions, below the required 100%. The repository's full feature acceptance
  requirements therefore remain unmet; this work does not mark phase 7 complete.

## Carousel update — 7 October 2026

Tests were updated first: 12 focused cases failed for missing single-photo
navigation, icon-only dismissal, and backdrop dismissal, while 38 passed.
After implementation the corresponding cases pass. Additional tests cover
keyboard boundaries, selection preservation during refresh, missing/empty
photos, four backdrop edges, ignored content/padding clicks, inside-to-outside
drags, and secondary-button clicks. The upload-to-gallery route regression now
navigates to pending images and verifies each becomes ready while it is active.

The frontend coverage command is
`npm --prefix web run test -- --coverage --coverage.reportsDirectory=/tmp/pinvoyager-gallery-carousel-coverage`.
All 475 frontend tests pass across 41 files with 100% statements, branches,
functions, and lines. Type checking, lint, production build, and diff whitespace
checks pass; existing warnings remain. API readiness and Vite respond successfully.
Backend code did not change, so backend tests and coverage were not rerun; the
previously recorded coverage shortfall remains. Browser-native dismissal,
focus behavior, and absence of scrollbars on real desktop/mobile browsers remain
unverified because the saved localhost browser preference still blocks automation.
