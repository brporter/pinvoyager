# Location marker photo gallery

Voyage page markers open a gallery of the photos represented by that marker.

## Behavior

Activating a single pin opens its photo. Activating a numbered cluster opens all photos
represented by that cluster at the current zoom, in voyage order. “Photos at this
location” refers to the photos grouped under the selected marker. The detail payload
already applies public coordinate precision, so the gallery never fetches owner-only
exact coordinates or introduces a new endpoint. Photos without coordinates are not
members of map markers and remain in the voyage's unplaced section. Discovery markers
navigate to a voyage or zoom into a cluster; see [map data
flow](architecture/map-data-flow.md).

`web/src/map/VoyageMap.tsx` (`VoyageMapProps.onOpenGallery`, `VoyageMarkers`,
`ClusterMarker`) sends marker member IDs to the page. On the Voyage page, clicking a
cluster opens its gallery. Zooming in can split the group into smaller markers. Without
a gallery callback, a cluster click zooms into that group. Single-pin selection still
highlights the corresponding photo. The desktop rail opens its photo page; the
mobile sheet scrolls it into view. Selecting a photo in the rail continues
to pan its pin.

`web/src/screens/VoyageDetail.tsx` (`VoyageDetail`) stores the selected IDs and filters
the current query payload in voyage order. This works with both desktop and mobile
layouts. It preserves the IDs while image processing updates the payload, so an open
gallery receives newly ready images through `useVoyage` polling. If photos disappear
from a refreshed payload, they disappear from the gallery too; the close action remains
available.

## Desktop voyage layout and photo pages

The desktop Voyage view (at widths of 768 px and above) fixes the map and photo
rail to the viewport height minus the 56 px application header. Adding photos
cannot increase the map height or the page height. The map attribution remains
visible. The mobile view continues to use its map and bottom sheet.

`web/src/screens/voyage/DesktopLayout.tsx` (`DesktopLayout`) measures the space
remaining below the voyage heading and above the photo navigation. It observes
rail size changes with `ResizeObserver` and also measures on window resize.
`web/src/screens/voyage/pagination.ts` (`paginatePhotoGroups`) fills each page
with complete two-column photo rows, accounting for date headings, grid padding,
and spacing between rows and sections. Date groups keep their existing order;
a group continuing on another page repeats its heading. Photos with unknown
dates remain in the Date unknown group. Photos without locations appear in a
trailing Photos with no location group and use the same pagination.

Only photos on the current page are mounted. **More** opens the following page;
**Previous** opens the preceding page. The first page omits Previous and the last
page omits More. A live page counter announces the current page and total.
Selecting a single map pin opens the page containing its photo and highlights
it, while continuing to open the location gallery. Desktop selection never
scrolls the document. Photo selection continues to pan the map.

Resizing repacks the rows and keeps the page containing the previously visible
first photo. If that photo disappears from refreshed data, the rail falls back
to the first page. On short screens thumbnails shrink to fit one row. An unusually
long voyage heading can scroll within its own area, limited to 45% of the rail;
photo pages themselves do not scroll. The API payload and public coordinate
precision are unchanged.

`web/src/styles/index.css` (`pv-voyage-desktop`, `pv-voyage-page-grid`,
`pv-voyage-pagination`) defines the fixed height and uses the existing design
tokens and photo components. `pagination.test.ts` checks row packing, heading
continuation, empty voyages, and insufficient space. The desktop photo pages
suite in `VoyageDetail.test.tsx` checks forward/backward navigation, end links,
pin selection, resizing, unplaced photos, and refreshed photo removal.

## HTML and visual treatment

`web/src/components/LocationGallery.tsx` (`LocationGallery`) uses:

- A native `<dialog>` opened by `showModal()` with a labeled heading.
- Browser-managed focus within the dialog, disabled interaction with the page
  behind it, and dismissal with Escape.
- A `<form method="dialog">` with an icon-only X submit button in the
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

`web/src/styles/index.css` (`pv-location-gallery`) uses shared colors, spacing, radii,
elevation, and focus styling. The modal fits inside the viewport and shows one image
contained within the available area, with navigation to either side. Its layout uses a
fixed header/footer and a flexible image frame with `min-height: 0` and `object-fit:
contain`. The dialog uses `overflow: hidden`, so gallery navigation does not require a
scrollbar. The same layout fits narrow screens. The backdrop uses `--pv-overlay`.

## Implementation tests

`web/src/map/__tests__/VoyageMap.test.tsx` checks marker membership and gallery callbacks.
`web/src/components/__tests__/LocationGallery.test.tsx` checks navigation, dismissal,
processing states, and lifecycle handling. `VoyageDetail.test.tsx` checks page integration
and focus restoration; `VoyageUploadFlow.test.tsx` checks readiness updates in an open gallery.
These jsdom tests model native dialog behavior; see [testing](testing.md) for their limits.
