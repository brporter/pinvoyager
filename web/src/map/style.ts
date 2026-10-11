/** Development raster basemap, attribution, and MapLibre camera limits. */

import type { StyleSpecification } from "@maplibre/maplibre-gl-style-spec";

/**
 * Where tiles come from.
 *
 * Overridable through `VITE_TILE_URL` so that pointing a developer at a local
 * tile server — or at the self-hosted archive, once it exists — needs no code
 * change. The template placeholders are MapLibre's: `{z}` zoom, `{x}` and `{y}`
 * the tile column and row.
 */
export const TILE_URL: string =
  import.meta.env.VITE_TILE_URL ?? "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/**
 * Highest zoom the tile source has tiles for.
 *
 * openstreetmap.org stops at 19. MapLibre will still zoom past this — it
 * upscales the level-19 tile rather than requesting a level-20 tile that would
 * 404 — which is what {@link MAX_ZOOM} then caps.
 */
const SOURCE_MAX_ZOOM = 19;

/**
 * How far the map lets a user zoom in.
 *
 * One level past the source, so the last step is a visibly blurry upscale
 * rather than a hard stop, and no further: beyond that the user is looking at
 * an enlargement of a tile, not at more detail.
 */
export const MAX_ZOOM = SOURCE_MAX_ZOOM + 1;

/** The tiles are a light-colored render. See the note above about D4a. */
export const BASEMAP_IS_DARK = false;

/**
 * The attribution text the tile source requires.
 *
 * MapLibre's own attribution control is switched off in `MapCanvas` because the
 * design draws this notice itself (`.pv-attribution`), so this string is
 * carried on the source for completeness and is not what the user reads. The
 * rendered notice is the `Attribution` component, and it is a license
 * requirement under ODbL rather than a courtesy.
 */
export const ATTRIBUTION = "© OpenStreetMap contributors";

/**
 * The style MapLibre is handed.
 *
 * `tileSize: 256` matches what openstreetmap.org serves. Getting it wrong does
 * not fail loudly — the map renders at the wrong scale, so every label is
 * half-size or double-size and the zoom levels no longer mean what they say.
 */
export function basemapStyle(tileUrl: string = TILE_URL): StyleSpecification {
  return {
    version: 8,
    sources: {
      basemap: {
        type: "raster",
        tiles: [tileUrl],
        tileSize: 256,
        maxzoom: SOURCE_MAX_ZOOM,
        attribution: ATTRIBUTION,
      },
    },
    layers: [
      {
        id: "basemap",
        type: "raster",
        source: "basemap",
      },
    ],
  };
}
