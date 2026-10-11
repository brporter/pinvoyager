/**
 * The map.
 *
 * | file                  | what it owns                                          |
 * |-----------------------|-------------------------------------------------------|
 * | `style.ts`            | the tile source — the one place a basemap swap touches |
 * | `geo.ts`              | projection, clustering, area sizing, camera fitting    |
 * | `MapCanvas.tsx`       | the MapLibre instance, camera, controls, attribution   |
 * | `VoyageMap.tsx`       | one voyage: pins, clusters, areas, selection           |
 * | `UploadReviewMap.tsx`  | upload batch: exact-coordinate pins and camera fitting |
 * | `PrecisionPreview.tsx` | St. Louis tiles and geographic precision examples |
 * | `DiscoverMap.tsx`     | every voyage: markers grouped by the server            |
 * | `Lazy*.tsx`           | the code-splitting boundaries                          |
 *
 * Screens import the `Lazy` components and nothing else. Importing
 * `VoyageMap` or `DiscoverMap` directly would compile, and would quietly pull
 * MapLibre into the entry chunk for every route in the application.
 *
 * The two maps group their markers in different places, and the difference is
 * not arbitrary. A voyage arrives whole, so the browser groups it and a pan
 * costs nothing. The world cannot arrive whole, so the server groups it and a
 * pan is a request.
 */

export { LazyUploadReviewMap } from "./LazyUploadReviewMap";
export { LazyVoyageMap } from "./LazyVoyageMap";
export { LazyDiscoverMap } from "./LazyDiscoverMap";
export type { DiscoverMapProps } from "./DiscoverMap";
export type { MapSelection, MapSelectionSource, VoyageMapProps, VoyagePin } from "./VoyageMap";
