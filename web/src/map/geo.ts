/**
 * Map geometry, as pure functions.
 *
 * Everything the map has to *calculate* lives here, and nothing here touches
 * MapLibre, the DOM, or React. That is deliberate: MapLibre cannot run in the
 * test environment at all — it needs a WebGL context, which jsdom does not
 * provide — so any arithmetic left inside the map component is arithmetic that
 * can only be checked by hand in a browser. Clustering, area sizing and camera
 * fitting are the three things most likely to be subtly wrong, and all three
 * are functions in this file with tests against them.
 *
 * ## The projection
 *
 * Web Mercator, the same projection MapLibre uses, expressed in **world
 * pixels**: the whole world is `256 * 2^zoom` pixels across at a given zoom.
 * Working in world pixels rather than screen pixels has one property that
 * matters a great deal here — the distance in pixels between two fixed
 * coordinates depends only on the zoom, never on where the camera is pointing.
 * So clustering can be computed from the pin list and a zoom number alone, and
 * a pan does not change the answer. That is what lets clustering be a pure
 * function instead of something that has to interrogate a live map.
 */

/** Web Mercator tile edge, in pixels, at zoom 0. */
const TILE_SIZE = 256;

/**
 * WGS84 equatorial circumference, in metres.
 *
 * Used for ground resolution. The value is the equatorial figure rather than a
 * mean, because Web Mercator's scale is defined at the equator and corrected by
 * `cos(latitude)` away from it.
 */
const EARTH_CIRCUMFERENCE_M = 40_075_016.686;

/**
 * The latitude Web Mercator stops at.
 *
 * The projection sends the poles to infinity, so every implementation cuts it
 * off somewhere; this is the standard cut, chosen because it makes the world
 * square. A coordinate past it is clamped rather than rejected — a photograph
 * taken in Antarctica is a real photograph, and refusing to draw it would be
 * worse than drawing it slightly too far north.
 */
export const MAX_MERCATOR_LATITUDE = 85.051129;

/** A geographic position. Structurally what the API's `Coordinate` is. */
export interface LngLat {
  latitude: number;
  longitude: number;
}

/** A position in world pixels at some zoom. */
export interface WorldPoint {
  x: number;
  y: number;
}

/** A geographic rectangle. */
export interface Bounds {
  west: number;
  south: number;
  east: number;
  north: number;
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

/** How many world pixels across the world is at this zoom. */
function worldSize(zoom: number): number {
  return TILE_SIZE * 2 ** zoom;
}

/**
 * Geographic position to world pixels.
 *
 * `y` grows southward, matching both the screen and MapLibre.
 */
export function project(coordinate: LngLat, zoom: number): WorldPoint {
  const scale = worldSize(zoom);
  const latitude = clamp(coordinate.latitude, -MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
  const sin = Math.sin((latitude * Math.PI) / 180);

  return {
    x: ((coordinate.longitude + 180) / 360) * scale,
    y: (0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * scale,
  };
}

/** World pixels back to a geographic position. The inverse of {@link project}. */
export function unproject(point: WorldPoint, zoom: number): LngLat {
  const scale = worldSize(zoom);

  return {
    longitude: (point.x / scale) * 360 - 180,
    latitude: (Math.atan(Math.sinh(Math.PI - (2 * Math.PI * point.y) / scale)) * 180) / Math.PI,
  };
}

/**
 * Ground resolution: how many metres one screen pixel covers.
 *
 * Latitude-dependent, and that is the whole point of it. Mercator stretches
 * east–west distance by `1 / cos(latitude)`, so a 10 km circle drawn in
 * Reykjavík is nearly twice as many pixels across as the same 10 km circle
 * drawn in Singapore. Sizing granularity areas with a single constant would
 * make the northern ones look far more precise than they are.
 */
export function metresPerPixel(latitude: number, zoom: number): number {
  const clamped = clamp(latitude, -MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
  return (EARTH_CIRCUMFERENCE_M * Math.cos((clamped * Math.PI) / 180)) / worldSize(zoom);
}

/**
 * The on-screen diameter, in pixels, of a granularity area.
 *
 * `precisionM` is the radius the server reports for the voyage's rounding —
 * `Coordinate.precision_m`, present only when the voyage is not at exact
 * precision — so the drawn circle is twice it. This is target 10: a voyage
 * rounded to ~10 km draws a circle 20 km across, and a viewer can see that the
 * photograph was taken somewhere inside it rather than at its centre.
 */
export function areaDiameterPx(precisionM: number, latitude: number, zoom: number): number {
  return (2 * precisionM) / metresPerPixel(latitude, zoom);
}

/**
 * The zoom at which a granularity area is `targetDiameterPx` across.
 *
 * Used to stop the camera zooming past what the coordinates actually know. A
 * voyage published at ~50 km precision with one photograph in it has a single
 * pin, and fitting a camera to a single point would otherwise zoom to street
 * level — showing a named junction for a position that is only known to within
 * fifty kilometres. Solving for the zoom that makes the *uncertainty* fill a
 * reasonable part of the viewport keeps the picture honest.
 *
 * Derived from {@link metresPerPixel}: given
 * `mpp = C·cos(lat) / (256·2^z)` and a wanted `mpp = 2·precision / target`,
 * `z = log2( C·cos(lat)·target / (256·2·precision) )`.
 */
export function zoomForPrecision(
  latitude: number,
  precisionM: number,
  targetDiameterPx: number,
): number {
  const clamped = clamp(latitude, -MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
  const numerator = EARTH_CIRCUMFERENCE_M * Math.cos((clamped * Math.PI) / 180) * targetDiameterPx;
  return Math.log2(numerator / (TILE_SIZE * 2 * precisionM));
}

/**
 * The rectangle containing every coordinate, or null for an empty list.
 *
 * Null rather than a zero-size rectangle at (0, 0), because "no pins" and "one
 * pin in the Gulf of Guinea" are different situations and the camera should
 * treat them differently — the first wants the world view, the second wants to
 * fly somewhere.
 *
 * No antimeridian handling: a voyage with photographs in both Fiji and Samoa
 * produces a rectangle spanning the long way round the planet, so the map shows
 * the whole world instead of the Pacific. That is a wrong-looking camera rather
 * than a wrong-looking pin, every pin is still in its correct place, and fixing
 * it properly means choosing between two legitimate readings of the same list.
 */
export function boundsOf(coordinates: readonly LngLat[]): Bounds | null {
  if (coordinates.length === 0) return null;

  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;

  for (const { latitude, longitude } of coordinates) {
    // A non-finite coordinate would poison every comparison and produce a
    // camera that cannot be fitted. Nothing currently rejects one at ingest,
    // which is recorded as an open issue against phase 4, so the map declines
    // to trust the data rather than assuming it.
    if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) continue;

    west = Math.min(west, longitude);
    east = Math.max(east, longitude);
    south = Math.min(south, latitude);
    north = Math.max(north, latitude);
  }

  if (west === Infinity) return null;
  return { west, south, east, north };
}

/* ==========================================================================
   Clustering
   ========================================================================== */

/** Anything with an identity and a position can be clustered. */
export interface Clusterable {
  id: string;
  coordinate: LngLat;
}

/**
 * A group of pins standing at one point.
 *
 * A group of one is still a cluster here. The caller draws a single-member
 * cluster as a pin and a multi-member cluster as a numbered bubble, but both
 * come out of the same list so nothing can fall out between the two cases.
 */
export interface MapCluster<T extends Clusterable> {
  /** The seed member's id. Stable across a zoom that does not change this
   *  cluster's membership, so React keeps the same DOM node and any focus
   *  inside it. */
  id: string;
  coordinate: LngLat;
  members: T[];
}

/**
 * How close two markers have to be, in pixels, before they merge.
 *
 * `--pv-touch-min`, the 44px touch target from the design system. Two markers
 * nearer than one touch target cannot be tapped apart on a phone, so leaving
 * them separate produces taps that appear to do nothing — which reads as the
 * application being broken rather than the finger being imprecise. That is the
 * same reasoning `.pv-pin` uses to put a 44px button around a 26px marker.
 */
export const CLUSTER_RADIUS_PX = 44;

/**
 * Groups pins that would overlap on screen.
 *
 * Greedy and seeded: walk the list in order, and each pin that has not yet been
 * taken starts a cluster and absorbs every remaining pin within
 * `radiusPx` **of the seed**. Absorbed pins do not go on to absorb their own
 * neighbours; without that restriction a line of pins each 40px from the next
 * would chain into one cluster spanning the continent, and the bubble would
 * appear somewhere none of the photographs were taken.
 *
 * The result is stable for a given pin order and zoom, which is what makes it
 * testable and what keeps markers from shuffling during a pan — the distance
 * between two fixed coordinates in world pixels does not depend on where the
 * camera is.
 *
 * Cost is quadratic in the number of pins. A voyage is a bounded collection —
 * tens to low hundreds of photographs — so this runs in well under a
 * millisecond, and it is recomputed only when the zoom changes, not on every
 * pan frame.
 */
export function clusterPins<T extends Clusterable>(
  pins: readonly T[],
  zoom: number,
  radiusPx: number = CLUSTER_RADIUS_PX,
): MapCluster<T>[] {
  const points = pins.map((pin) => ({ pin, world: project(pin.coordinate, zoom) }));
  const taken = new Array<boolean>(points.length).fill(false);
  const clusters: MapCluster<T>[] = [];
  const radiusSquared = radiusPx * radiusPx;

  for (let i = 0; i < points.length; i += 1) {
    if (taken[i]) continue;

    const seed = points[i];
    taken[i] = true;

    const members = [seed.pin];
    let sumX = seed.world.x;
    let sumY = seed.world.y;

    for (let j = i + 1; j < points.length; j += 1) {
      if (taken[j]) continue;

      const dx = points[j].world.x - seed.world.x;
      const dy = points[j].world.y - seed.world.y;
      if (dx * dx + dy * dy > radiusSquared) continue;

      taken[j] = true;
      members.push(points[j].pin);
      sumX += points[j].world.x;
      sumY += points[j].world.y;
    }

    clusters.push({
      id: seed.pin.id,
      // Averaged in world pixels and projected back, not averaged as latitude
      // and longitude. Mercator is not linear in latitude, so a bubble placed
      // by averaging degrees would sit visibly off-centre from the pins it
      // stands for at high latitudes.
      coordinate:
        members.length === 1
          ? seed.pin.coordinate
          : unproject({ x: sumX / members.length, y: sumY / members.length }, zoom),
      members,
    });
  }

  return clusters;
}

/**
 * Which of the design's three bubble sizes a count gets.
 *
 * `design/components/map-primitives.html` draws the tiers with 7, 24 and 156,
 * so the boundaries are the powers of ten those examples sit either side of.
 * The purpose is that a glance distinguishes "a handful" from "a lot" without
 * reading the number.
 */
export function clusterTier(count: number): "sm" | "md" | "lg" {
  if (count >= 100) return "lg";
  if (count >= 10) return "md";
  return "sm";
}
