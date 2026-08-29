/**
 * The MapLibre instance, and everything that has to touch it.
 *
 * This is the only file in the application that imports `maplibre-gl`. It is
 * loaded lazily — see `VoyageMap.tsx` — so MapLibre and its stylesheet are a
 * separate bundle that no route pays for until a map is actually on screen
 * (target 12). MapLibre plus a style is a substantial download, and putting it
 * in the entry chunk would delay first paint on every screen including the ones
 * with no map at all.
 *
 * ## What it does and does not own
 *
 * It owns the camera, the tile source, the zoom controls and the attribution.
 * It owns no knowledge of voyages, photographs, pins or selection; those are
 * `VoyageMap.tsx`. The two are separated because the camera and the tile source
 * are the parts that change when the basemap is swapped for self-hosted vector
 * tiles, and the pin behavior is the part that must not have to change with
 * it.
 *
 * ## Markers rather than a symbol layer
 *
 * Pins and clusters are real DOM elements positioned by {@link MapMarker},
 * not features drawn into the WebGL canvas by a MapLibre style layer. The
 * canvas approach is faster for tens of thousands of points, and it is the
 * wrong choice here for two reasons:
 *
 *  * A pin drawn on a canvas is not a button. It cannot be tabbed to, it has no
 *    accessible name, and it cannot carry the 44px hit area that
 *    `design/components/map-primitives.html` insists on. Every one of those is
 *    a requirement the design states outright.
 *  * The pin, cluster and area treatments already exist as CSS in
 *    `design/components.css`. Re-expressing them as MapLibre paint properties
 *    would make the design system a description of the map rather than its
 *    source, which is the exact drift phase 6 target 2 exists to prevent.
 *
 * A voyage holds tens to low hundreds of photographs, which is comfortably
 * inside what DOM markers handle, and clustering keeps the count on screen
 * lower still.
 */

import * as maplibregl from "maplibre-gl";
import type { Map as MapLibreMap, Marker } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { Attribution, cx } from "../components";
import { cameraDuration } from "../shell/motion";
import type { Bounds, LngLat } from "./geo";
import { BASEMAP_IS_DARK, MAX_ZOOM, basemapStyle } from "./style";

/* ==========================================================================
   Context
   ========================================================================== */

const MapContext = createContext<MapLibreMap | null>(null);

/* ==========================================================================
   Canvas
   ========================================================================== */

/** What the children of a {@link MapCanvas} are told about the camera. */
export interface MapView {
  /**
   * The current zoom, fractional.
   *
   * Both things drawn on top of the basemap depend on it: which pins are close
   * enough to merge into a cluster, and how many pixels across a granularity
   * area is. Neither depends on where the camera is pointing, only on how far
   * in it is — see the note in `geo.ts` about working in world pixels.
   */
  zoom: number;

  /**
   * Moves the camera, honoring the reduced-motion preference.
   *
   * Offered here rather than as an exported hook so that nothing outside this
   * file needs the MapLibre instance to move the map. That is what keeps the
   * pin and selection behavior independent of the mapping library: swapping
   * MapLibre out means rewriting this file, not every caller.
   *
   * `zoom` is absolute, not a delta, and omitting it leaves the zoom alone.
   */
  moveTo: (coordinate: LngLat, options?: { zoom?: number }) => void;

  /**
   * The rectangle currently on screen, or null before the map has laid itself
   * out and knows how big it is.
   *
   * Updated when a gesture *finishes*, not while it is happening. The
   * discovery map turns this into a request, and a request per animation frame
   * of a pan would be both useless and a breach of the tile server's usage
   * policy in spirit — see decision D4a on not fetching beyond what the user is
   * looking at.
   */
  window: Bounds | null;
}

export interface MapCanvasProps {
  /**
   * Names the map for assistive technology. Required — a map is a graphic, and
   * "map" alone does not distinguish this one from any other.
   */
  label: string;
  /**
   * Where the camera should sit. Null leaves the world view, which is the right
   * answer for a voyage with no placed photographs: there is nowhere to fly to.
   */
  fitTo?: Bounds | null;
  /**
   * Never zoom in past this when fitting. Callers derive it from the voyage's
   * precision so the camera does not claim detail the coordinates do not have.
   */
  maxFitZoom?: number;
  /**
   * Pixels to lift the attribution by, so the mobile sheet does not cover it.
   * The notice is a license requirement, so it is not allowed to slide out of
   * sight behind another element.
   */
  attributionOffset?: number;
  className?: string;
  style?: CSSProperties;
  /**
   * Rendered once the map has loaded, and given the current camera. A render
   * prop rather than plain children because markers cannot be created before
   * there is a map to add them to.
   */
  children?: (view: MapView) => ReactNode;
}

/** Pixels of clear space left around the fitted bounds, so a pin at the extreme
 *  of a voyage is not drawn half off the edge. */
const FIT_PADDING_PX = 64;

/** How long a camera movement takes when motion is not reduced. */
const FIT_DURATION_MS = 600;

/** A step of the zoom buttons. */
const ZOOM_STEP_DURATION_MS = 250;

/**
 * Concurrent tile requests.
 *
 * MapLibre's default is 16. Lowered because decision D4a points development at
 * openstreetmap.org's tile server, whose usage policy is explicit that heavy
 * use is not permitted, and a single pan gesture across a city can ask for
 * dozens of tiles. Six is what a browser will open to one origin anyway, so
 * this caps the burst without making an ordinary pan feel slow.
 */
const MAX_PARALLEL_TILE_REQUESTS = 6;

export function MapCanvas({
  label,
  fitTo = null,
  maxFitZoom = MAX_ZOOM,
  attributionOffset = 0,
  className,
  style,
  children,
}: MapCanvasProps) {
  const container = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<MapLibreMap | null>(null);
  const [zoom, setZoom] = useState(0);
  const [visible, setVisible] = useState<Bounds | null>(null);

  /**
   * The bounds the camera has already been sent to.
   *
   * Set to the initial bounds when the map is constructed, because those are
   * applied through the constructor rather than by a call — starting at the
   * world view and then flying to the voyage would download a set of tiles the
   * user never looks at, which is the pre-fetching that D4a records
   * openstreetmap.org's policy as forbidding.
   */
  const appliedBounds = useRef<string | null>(null);
  const boundsKey = keyOf(fitTo);

  useEffect(() => {
    const node = container.current;
    if (node === null) return;

    maplibregl.setMaxParallelImageRequests(MAX_PARALLEL_TILE_REQUESTS);

    const instance = new maplibregl.Map({
      container: node,
      style: basemapStyle(),
      // The design draws its own notice (`.pv-attribution`) and this component
      // renders it below. Leaving MapLibre's control on as well would show the
      // same attribution twice.
      attributionControl: false,
      maxZoom: MAX_ZOOM,
      // Rotation and tilt are off. A travelogue map is read, not flown through,
      // and a map that has silently rotated is one the reader cannot recover
      // north on without a control the design does not have.
      dragRotate: false,
      pitchWithRotate: false,
      touchZoomRotate: true,
      ...(fitTo === null
        ? { center: [0, 20] as [number, number], zoom: 1 }
        : {
            bounds: boundsOfMapLibre(fitTo),
            fitBoundsOptions: { padding: FIT_PADDING_PX, maxZoom: maxFitZoom },
          }),
    });

    instance.touchZoomRotate.disableRotation();
    appliedBounds.current = boundsKey;

    const onZoom = () => setZoom(instance.getZoom());

    const onSettled = () => {
      const bounds = instance.getBounds();
      const next: Bounds = {
        west: bounds.getWest(),
        south: bounds.getSouth(),
        east: bounds.getEast(),
        north: bounds.getNorth(),
      };

      // Compared by value. A camera movement that ends where it began still
      // fires `moveend`, and handing React a new-but-equal object each time
      // would re-render every marker for a rectangle that has not moved.
      setVisible((current) => (keyOf(current) === keyOf(next) ? current : next));
    };

    // `load` fires once the style and the first tiles are in. Markers are only
    // rendered after it, because adding one to a map that has not finished
    // loading positions it against a camera that is about to change.
    instance.on("load", () => {
      setZoom(instance.getZoom());
      onSettled();
      setMap(instance);
    });
    instance.on("zoom", onZoom);
    instance.on("moveend", onSettled);

    return () => {
      setMap(null);
      instance.remove();
    };
    // Constructed once. `fitTo` and `maxFitZoom` are read here only to set the
    // opening camera; later changes are handled by the effect below, which
    // moves the existing camera rather than rebuilding the map. Listing them
    // would rebuild the whole map — and re-download every tile — each time a
    // voyage's pins shifted.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (map === null || fitTo === null) return;
    if (appliedBounds.current === boundsKey) return;

    appliedBounds.current = boundsKey;
    map.fitBounds(boundsOfMapLibre(fitTo), {
      padding: FIT_PADDING_PX,
      maxZoom: maxFitZoom,
      // MapLibre animates its camera in JavaScript, outside CSS, so the design
      // system's reduced-motion rules do not reach it. The preference has to be
      // passed in by hand or the map keeps flying for the users who asked it
      // not to.
      duration: cameraDuration(FIT_DURATION_MS),
    });
    // `fitTo` is compared by value through `boundsKey`, which is in the list.
    // Depending on the object itself would re-fly the camera on every render
    // that happened to build an equal-but-new one.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, boundsKey, maxFitZoom]);

  /**
   * Stable for the lifetime of the map instance, and that is load-bearing
   * rather than tidy.
   *
   * Callers put `moveTo` in an effect's dependency list — panning to a
   * photograph chosen in the list is exactly that shape. If its identity
   * changed whenever the camera moved, the sequence would be: the effect eases
   * the camera, the camera settles, `moveend` updates this component's state,
   * the callback is rebuilt, the effect's dependencies have changed, and the
   * effect eases the camera again. That is an endless flight, and it is not a
   * test artifact — `moveend` fires in a browser for the same reason.
   */
  const moveTo = useCallback(
    (coordinate: LngLat, options?: { zoom?: number }) => {
      map?.easeTo({
        center: [coordinate.longitude, coordinate.latitude],
        ...(options?.zoom === undefined ? {} : { zoom: options.zoom }),
        duration: cameraDuration(FIT_DURATION_MS),
      });
    },
    [map],
  );

  const view = useMemo<MapView>(
    () => ({ zoom, window: visible, moveTo }),
    [zoom, visible, moveTo],
  );

  return (
    <div className={cx("pv-map", className)} style={style} role="region" aria-label={label}>
      {/* MapLibre owns everything inside this element and replaces its
          contents, so the controls and the attribution are siblings of it
          rather than children. */}
      <div ref={container} style={{ position: "absolute", inset: 0 }} />

      <MapContext.Provider value={map}>{map !== null && children?.(view)}</MapContext.Provider>

      {/* Kept on screen even though pinch and scroll zoom both work: a pointer
          user with no wheel, and a keyboard user, need a non-gesture path to
          the same capability. Both are 44px, per the design. */}
      <div className="pv-map-controls">
        <button
          type="button"
          className="pv-map-ctrl"
          aria-label="Zoom in"
          onClick={() => map?.zoomIn({ duration: cameraDuration(ZOOM_STEP_DURATION_MS) })}
        >
          +
        </button>
        <button
          type="button"
          className="pv-map-ctrl"
          aria-label="Zoom out"
          onClick={() => map?.zoomOut({ duration: cameraDuration(ZOOM_STEP_DURATION_MS) })}
        >
          −
        </button>
      </div>

      {/* A zero-height box for the notice to anchor to, which is how it is
          lifted clear of the mobile sheet without touching the design system's
          own `.pv-attribution` rule. */}
      <div
        style={{ position: "absolute", left: 0, right: 0, bottom: attributionOffset, height: 0 }}
      >
        <Attribution dark={BASEMAP_IS_DARK} />
      </div>
    </div>
  );
}

/* ==========================================================================
   Markers
   ========================================================================== */

export interface MapMarkerProps {
  coordinate: LngLat;
  /**
   * Drawn at the coordinate. Positioned by the design system's own rules — see
   * the note on the anchor element below — so a `.pv-pin` puts its visible tip
   * on the point and a `.pv-cluster` centers its bubble on it.
   */
  children: ReactNode;
}

/**
 * One thing on the map, at one coordinate.
 *
 * MapLibre positions the anchor element; React renders into it through a
 * portal. The division matters for performance: MapLibre rewrites the
 * element's `transform` on every frame of a pan directly, without React
 * involved, so panning a voyage with a hundred pins does not run a hundred
 * component renders per frame.
 *
 * The anchor element is deliberately **zero-sized**. MapLibre centers it on the
 * coordinate, and a zero-sized box's center is its top-left, so the coordinate
 * ends up at the element's origin. Everything drawn inside then positions
 * itself with the same `position: absolute` and negative margins the design
 * system already uses on `.pv-map` — `.pv-pin` offsets by half its width and
 * its full height to stand its point on the spot, `.pv-cluster` by half its
 * diameter in both directions, `.pv-map-area` by its own `translate(-50%,
 * -50%)`. No marker geometry is restated here, so the pin that appears on a
 * real map is positioned by the same rules as the pin in the design specimen.
 */
export function MapMarker({ coordinate, children }: MapMarkerProps) {
  const map = useContext(MapContext);

  const [element] = useState(() => {
    const node = document.createElement("div");
    node.style.position = "absolute";
    node.style.width = "0";
    node.style.height = "0";
    return node;
  });

  const marker = useRef<Marker | null>(null);

  useEffect(() => {
    if (map === null) return;

    const created = new maplibregl.Marker({ element, anchor: "center" })
      .setLngLat([coordinate.longitude, coordinate.latitude])
      .addTo(map);
    marker.current = created;

    return () => {
      marker.current = null;
      created.remove();
    };
    // The coordinate is not a dependency on purpose. A marker that moves is
    // repositioned by the effect below; tearing it down and rebuilding it would
    // destroy the portal's host element, which throws away any focus inside it
    // — so a keyboard user whose selected pin shifted by a meter would lose
    // their place on the map.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, element]);

  useEffect(() => {
    marker.current?.setLngLat([coordinate.longitude, coordinate.latitude]);
  }, [coordinate.longitude, coordinate.latitude]);

  return createPortal(children, element);
}

/* ==========================================================================
   Helpers
   ========================================================================== */

/** MapLibre wants a south-west / north-east pair. */
function boundsOfMapLibre(bounds: Bounds): [[number, number], [number, number]] {
  return [
    [bounds.west, bounds.south],
    [bounds.east, bounds.north],
  ];
}

/** Compares bounds by value, so a re-render that produces an equal-but-new
 *  object does not re-fly the camera. */
function keyOf(bounds: Bounds | null): string | null {
  if (bounds === null) return null;
  return `${bounds.west},${bounds.south},${bounds.east},${bounds.north}`;
}
