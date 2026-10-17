/**
 * Map entry points and supporting modules.
 *
 * Screens import Lazy components to defer MapLibre loading. MapCanvas owns
 * the library instance; geo.ts computes geometry; style.ts supplies the basemap.
 * Voyage maps cluster in the browser; discovery maps use server clusters.
 * Upload review uses exact owner coordinates, and precision preview illustrates
 * the selected rounding scale.
 */

export { LazyUploadReviewMap } from "./LazyUploadReviewMap";
export { LazyVoyageMap } from "./LazyVoyageMap";
export { LazyDiscoverMap } from "./LazyDiscoverMap";
export type { DiscoverMapProps } from "./DiscoverMap";
export type { MapSelection, MapSelectionSource, VoyageMapProps, VoyagePin } from "./VoyageMap";
