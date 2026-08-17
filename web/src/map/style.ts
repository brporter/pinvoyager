/**
 * The basemap style — the single place the tile source is decided.
 *
 * ## Why this file exists at all
 *
 * Phase 7 target 4 asks that swapping the tile source later be a change to one
 * style definition rather than to the map code. Nothing outside this file names
 * a tile URL, a tile scheme, or a source id. `MapCanvas` asks for
 * {@link basemapStyle} and does not know whether it got raster or vector tiles
 * back.
 *
 * ## What it currently returns, and why that is temporary
 *
 * `https://tile.openstreetmap.org/{z}/{x}/{y}.png` — raster tiles, straight
 * from openstreetmap.org, which is a development-only arrangement recorded as
 * decision D4a in `docs/decisions.md`. Their usage policy permits light and
 * development use; it does not permit a released application, so shipping this
 * URL would be a breach of it rather than merely impolite.
 *
 * The consequence that shows up visually: raster tiles arrive already drawn.
 * There is no way to recolour them, so the dark basemap in
 * `design/components/map-primitives.html` is unreachable until the self-hosted
 * vector archive that decision D4 describes is in place. The map is a
 * light-coloured OpenStreetMap render regardless of the page theme, and the
 * pins and clusters sit on top of it.
 *
 * ## Switching to vector tiles later
 *
 * Replace the body of {@link basemapStyle} with a style whose sources are
 * `{ type: "vector", tiles: [...] }` and whose layers draw land, water, roads
 * and labels from the design tokens. The exported name, its signature, and
 * {@link BASEMAP_IS_DARK} are the whole contract; everything else in
 * `src/map/` is written against those.
 */

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

/** The tiles are a light-coloured render. See the note above about D4a. */
export const BASEMAP_IS_DARK = false;

/**
 * The attribution text the tile source requires.
 *
 * MapLibre's own attribution control is switched off in `MapCanvas` because the
 * design draws this notice itself (`.pv-attribution`), so this string is
 * carried on the source for completeness and is not what the user reads. The
 * rendered notice is the `Attribution` component, and it is a licence
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
