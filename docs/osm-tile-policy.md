# Development map tile requests

All maps use the raster style in [`web/src/map/style.ts`](../web/src/map/style.ts)
through [`MapCanvas`](../web/src/map/MapCanvas.tsx). The default tile URL is
`https://tile.openstreetmap.org/{z}/{x}/{y}.png`. Requests go directly from the
browser to the tile server; the API does not proxy or cache tiles.

`MapCanvas` configures normal browser caching, same-origin credentials, and
`strict-origin` referrer policy. External tile requests therefore carry the site
origin rather than a voyage path/query, and do not carry application credentials.
The browser supplies its User-Agent. MapLibre retains expired-tile refresh behavior.
There is no bulk download, offline archive, or all-precision preview prefetch.

`MapPrimitives.Attribution` renders a visible OpenStreetMap copyright link.
The mobile voyage layout offsets attribution above its photo sheet. Attribution
styles come from `design/components.css`; the same component is used by discovery,
voyage, precision preview, and upload review maps.

The project limits this source to development. The service's own usage terms are
published in the [OpenStreetMap tile policy](https://operations.osmfoundation.org/policies/tiles/).
The repository's development restriction is a project constraint, separate from
the provider's terms. Changing the tile URL changes the applicable provider terms.

## Implementation checks

`web/src/map/__tests__/MapCanvas.test.tsx` checks the default style, request options,
attribution, and map lifecycle. `PrecisionPreview.test.tsx` checks that the selector
uses the shared map configuration. These use a MapLibre substitute; actual network
requests and rendered attribution visibility require a browser check. See
[testing](testing.md) and [map data flow](architecture/map-data-flow.md).
