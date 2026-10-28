# Implementation constraints

These constraints describe required behavior and limits in the current system. The D
identifiers match references in source comments. See the [architecture
overview](../architecture.md) for modules, routes, and data flow.

| Source identifier | Required behavior |
| --- | --- |
| D1 | Authentication is delegated to providers. The application stores no passwords or long-lived provider access/refresh tokens. Session hashes, short-lived authorization attempts, and configured provider secrets exist. |
| D2 | A verified provider identity signs in to its existing account or creates an account. There is no separate password signup path. |
| D3 | Identities match on provider and subject, never email. Linking another provider requires an authenticated account. |
| D4, D4a | MapLibre uses OpenStreetMap raster tiles for development. Attribution is visible and requests use normal browser caching and an origin Referer. This tile configuration is normally limited to development by project policy; the owner authorized it for the initial production deployment on 2026-10-11 (see the tile policy reference). |
| D5 | The application performs no geocoding. Place context comes from basemap labels. |
| D6 | Published voyages are public; drafts return 404 to non-owners. The administrator-only account detail endpoint may list draft titles/descriptions, but does not grant draft photo access. Public voyage coordinates use the voyage's granularity. Exact coordinates remain available in owner-only photo metadata. Originals are never served. |
| D7 | Image paths derive from content hashes. The authorized HTTP handler serves only re-encoded WebP variants with controlled response headers. Variant responses are private and must revalidate authorization before reuse. The native owner-only offline-download contract separately permits explicit protected copies of re-encoded variants; it never grants original-download access. |
| D8 | Database access is async. CPU-heavy image work and blocking file operations run off Tokio worker threads. The database pool has an acquisition timeout. |
| D9 | JPEG/PNG/WebP decoding uses `image`, HEIC uses native libheif, and lossy WebP encoding uses libwebp. All variants are generated from decoded pixels without copied metadata. |
| D10 | Voyage pins are grouped in the browser. Discovery groups public rounded positions on the server after rounding coordinates and checking whether they fall inside the viewport. |
| D11 | Every photo belongs to one voyage with the same owner. The creation UI requires name and description before upload and defaults precision to approximately 1 km. Owners can choose another precision. Upload appends photos; API endpoints support ordering and deletion. Publication is a separate owner action. |
| D12 | Voyage marker activation opens a native dialog containing that marker's photos in voyage order. Discovery markers retain their navigation/zoom behavior. |
| D13 | Optional OpenTelemetry traces carry request and image-job context. Development export uses an OTLP HTTP Collector and transient Jaeger storage. |
| D14 | Profiles use `/u/:identifier` with account IDs or display names, separate from top-level system routes. Claims are unique and lowercase; old names remain aliases and deleted names stay reserved. Only the signed-in owner sees drafts; all other profile visitors see published voyages. |

The [image pipeline](../photo-pipeline.md), [map flow](map-data-flow.md),
[photo ownership](../voyage-owned-photos.md), and [display-name reference](../display-names.md)
describe implementation details and current limitations.
