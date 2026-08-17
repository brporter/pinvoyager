/**
 * The map.
 *
 * | file                | what it owns                                          |
 * |---------------------|-------------------------------------------------------|
 * | `style.ts`          | the tile source — the one place a basemap swap touches |
 * | `geo.ts`            | projection, clustering, area sizing, camera fitting    |
 * | `MapCanvas.tsx`     | the MapLibre instance, camera, controls, attribution   |
 * | `VoyageMap.tsx`     | pins, clusters, granularity areas, selection           |
 * | `LazyVoyageMap.tsx` | the code-splitting boundary                            |
 *
 * Screens import {@link LazyVoyageMap} and nothing else. Importing `VoyageMap`
 * directly would compile, and would quietly pull MapLibre into the entry chunk
 * for every route in the application.
 */

export { LazyVoyageMap } from "./LazyVoyageMap";
export type {
  MapSelection,
  MapSelectionSource,
  VoyageMapProps,
  VoyagePin,
} from "./VoyageMap";
