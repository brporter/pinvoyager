# OSM raster tile policy audit

Reviewed 6 October 2026 against the official
[Tile Usage Policy](https://operations.osmfoundation.org/policies/tiles/).
This audit covers the default `tile.openstreetmap.org` raster source. A
`VITE_TILE_URL` override requires a separate check of that provider and its policy.
The separate OSM vector service is not used.

## Request path and evidence

`web/src/map/style.ts` (`TILE_URL`, `basemapStyle`) declares the only production
tile URL. `web/src/map/MapCanvas.tsx` (`MapCanvas`) constructs the MapLibre map
used by both discovery and voyage maps. Tiles travel directly from the browser
to OSM; `web/vite.config.ts` proxies only our `/api` and `/auth` requests.
The Rust API has no OSM tile client or tile proxy.

`MapCanvas` sets `cache: "default"`, `credentials: "same-origin"`, and
`referrerPolicy: "strict-origin"` on resource requests. Cross-origin OSM requests
therefore carry no credentials, and their Referer contains the site origin rather
than a voyage path or query string. It leaves User-Agent and HTTP cache validation
to the browser. It explicitly enables `refreshExpiredTiles`.

The installed MapLibre implementation was also inspected:

- `web/node_modules/maplibre-gl/src/util/request_manager.ts`,
  `RequestManager.transformRequest`, passes the configured request settings on.
- `web/node_modules/maplibre-gl/src/util/ajax.ts`, `getReferrer` and
  `makeFetchRequest`, constructs a browser Request with those settings and reads
  Cache-Control, Expires, and ETag from the response.
- `web/node_modules/maplibre-gl/src/util/image_request.ts`,
  `ImageRequest.transformAndGetImage` and `doImageRequest`, uses Fetch/XHR for
  raster images when expiry refresh is enabled.
- `web/node_modules/maplibre-gl/src/source/raster_tile_source.ts`,
  `RasterTileSource.loadTile`, passes expiry data to the tile cache.

These dependency references describe the installed version; recheck them when
upgrading MapLibre. Automated tests mock MapLibre and never contact OSM.

## Rules and recommendations

“Code checked” means source and tests support the finding. It does not mean a
deployed browser session was observed. “Operational” requires continued attention
outside this code. Recommendations are distinguished from mandatory requirements.

| OSM requirement or recommendation | Finding and how PinVoyager addresses it |
| --- | --- |
| Required: use the documented HTTPS endpoint (§1). | Code checked: `TILE_URL` defaults to `https://tile.openstreetmap.org/{z}/{x}/{y}.png`. No alternate OSM host, HTTP scheme, or URL parameters are added. The raster source ends at zoom 19. |
| Required: visible, legible attribution linked to licensing information (§2). | Code checked: `MapPrimitives.Attribution` renders the copyright notice and links to `https://www.openstreetmap.org/copyright`. `MapCanvas` renders it on every map; `VoyageDetail.MobileDetail` raises it above the photo sheet. `design/components.css` supplies a contrasting background. Actual visibility at all device sizes still needs browser inspection. |
| Required: browser identification; distinct app identification for non-browser clients (§3.1, §3.4). | Code checked: real browser requests use the browser User-Agent. There is no server tile client, SDK, User-Agent override, or impersonation. The origin Referer identifies the website. Any future server client needs its own identifiable User-Agent. |
| Required: preserve a valid Referer; do not suppress it through referrer policy (§3.1, §3.4). | Code checked: the request explicitly uses `strict-origin`. Normal HTTP/HTTPS page loads send the site origin to HTTPS OSM. No tile proxy strips it. Privacy extensions, browser settings, and deployment behavior require an actual network check. |
| Required: stable identification, without spoofing or rotation (§3.4). | Code checked: the app does not replace or randomize identification. Browser identity and the current site origin are supplied by the platform. |
| Required: obey cache expiry headers and keep a useful local cache (§3.2). | Code checked: `cache: "default"` uses the browser HTTP cache; MapLibre reads expiry headers with `refreshExpiredTiles: true`. No application cache overrides expiry or clears the browser cache. Storage availability and eviction depend on the browser. |
| Required: seven-day retention when a cache cannot interpret response headers (§3.2). | Not applicable to this implementation: it relies on the browser HTTP cache, which understands the headers, rather than a custom cache without header support. |
| Required: conditional validation for expired cached entries (§3.2). | Browser-managed: default HTTP caching allows validator-based requests using stored ETag/Last-Modified values. The app does not manually replace validators. Whether a particular response has validators must be checked in browser traffic. |
| Required: no default cache-bypass headers (§3.2). | Code checked: the request adds no Cache-Control or Pragma headers and does not use a reload/no-store cache mode. Users should leave DevTools “Disable cache” off during normal map testing. |
| Required: no bulk collection, region/zoom pre-seeding, tile archives, or offline-download features (§4). | Code checked: no downloader, service worker tile cache, archive writer, background scan, or offline UI exists. Lazy map components instantiate MapLibre when the map is mounted. Tests use a mock and do not pan a real map to download tiles. |
| Allowed: normal viewport tiles and modest nearby look-ahead (§4). | Code checked: MapLibre selects tiles for the displayed map. The opening voyage bounds are applied in the constructor rather than fetching a world view first. Later camera changes support user viewing; there is no separate prefetch loop. |
| Required when proxying: retain identification and obey caching rules (§3.4, §5). | Not applicable: OSM traffic is not proxied through Rust, Vite, or a project CDN. Reaudit before introducing any tile proxy. |
| Required: do not encourage copyright infringement (§2). | No infringement or copying facility was found in the tile integration. OSM attribution remains present. The app cannot prove users have rights to every uploaded photo; content handling is an ongoing responsibility under the broader Terms of Use. |
| Required: do not submit personal or confidential material to OSM (§6). | Code checked for request payloads: tile requests contain tile indices, not uploaded photos, EXIF, account IDs, session credentials, voyage IDs, or private page paths. OSM still sees the visitor's IP, origin, and viewed tile areas. Sensitive location viewing cannot be made confidential while using a third-party basemap; this is a privacy limitation, not a claim that OSM learns no location information. |
| Operational: avoid traffic that degrades the shared service; access is best-effort and may be withdrawn (introduction, §7). | `MapCanvas` limits concurrent image requests to six and reuses its map instance during camera changes. Six is our choice, not an OSM quota or proof of acceptable aggregate load. Monitor usage and switch providers if OSM requests it or traffic becomes unsuitable. |
| Recommended: make the source switchable. | Partly met: `VITE_TILE_URL` changes the source without editing code, but Vite embeds it at build time, so changing a deployed source still requires a new build. There is no runtime configuration endpoint. |
| Recommended: link to OSM's map-issue reporting page. | Not implemented: the current attribution links to licensing only. A report link to `https://www.openstreetmap.org/fixthemap` remains optional follow-up. |
| Recommended: publish a contact email. | Not established: no public operator contact email was found in the reviewed app. The owner must choose an address before it can be published. |
| Recommended: use HTTP/2 or HTTP/3 (§3.3). | Browser-managed: direct HTTPS traffic lets the browser and OSM negotiate their supported protocol. The app does not force HTTP/1. Actual negotiation needs a network inspection. |
| Scope: other providers and OSM's vector service have separate policies (§9). | This audit applies only to the default standard raster source. Do not infer that it approves an overridden URL or future vector provider. |

## Checks and limitations

`web/src/map/__tests__/MapCanvas.test.tsx` verifies the exact default source,
request cache/referrer/credential settings, absence of custom headers, and
expiry refresh. It also constructs a real platform Request to validate the
settings, without making a network call. Existing voyage/discovery tests check
attribution, and mobile voyage tests exercise the sheet layout.

The new request-settings test failed before implementation because no
`transformRequest` existed, then passed after implementation. Checks on 6 October
2026:

- `npm run test -- --coverage --coverage.reportsDirectory=/tmp/pinvoyager-osm-coverage`:
  444 tests passed across 39 files. Statements, branches, functions, and lines
  each remained at 100%.
- `npm run typecheck`, `npm run lint`, and `npm run build`: passed. Lint reports
  existing Fast Refresh warnings; the build reports existing configuration and
  chunk-size warnings.
- `git diff --check`: passed.
- `cargo llvm-cov report --fail-under-lines 100 --fail-under-functions 100 --fail-under-regions 100`:
  failed the thresholds using the existing collected backend profiles, with
  98.12% lines, 96.32% functions, and 95.09% regions. Backend source is unchanged
  and backend tests were not rerun for this frontend change. This audit does not
  declare the project complete under the 100% coverage requirement.

Live browser network inspection was not performed: earlier browser access to the
local app was denied. This is a code/dependency audit, not certification of every
request in an arbitrary deployment. Before deployment, check a normal manual map
view in browser DevTools with cache enabled: exact OSM URL, origin Referer,
browser User-Agent, no default cache-bypass headers, caching/revalidation on
repeat views, negotiated protocol, and attribution unobscured on desktop and
mobile. Do not bulk-pan or automate scans against OSM to verify this.

Official references:

- [Tile Usage Policy](https://operations.osmfoundation.org/policies/tiles/)
- [Identification and caching](https://operations.osmfoundation.org/policies/tiles/#3-technical-requirements)
- [Bulk/offline restrictions and allowed interactive use](https://operations.osmfoundation.org/policies/tiles/#4-prohibited-bulk-downloading-scraping-and-offline-use)
- [Privacy](https://operations.osmfoundation.org/policies/tiles/#6-privacy)
- [Enforcement and availability](https://operations.osmfoundation.org/policies/tiles/#7-changes-and-enforcement)
- [Attribution guidelines](https://osmfoundation.org/wiki/Licence/Attribution_Guidelines)
- [OSMF Terms of Use](https://osmfoundation.org/wiki/Terms_of_Use)
