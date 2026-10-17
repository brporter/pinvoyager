/**
 * Pure geometry for projection, bounds, clustering, and precision display.
 * Uses Web Mercator world pixels: world width is 512 * 2^zoom. Distances at a
 * given zoom are independent of camera position, so panning does not require
 * recomputing voyage clusters.
 */

/** Web Mercator tile edge, in pixels, at zoom 0. */
const TILE_SIZE = 512;

/**
 * WGS84 equatorial circumference, in meters.
 *
 * Used for ground resolution. The value is the equatorial figure rather than a
 * mean, because Web Mercator's scale is defined at the equator and corrected by
 * `cos(latitude)` away from it.
 */
const EARTH_CIRCUMFERENCE_M = 40_075_016.686;

/**
 * Maximum projected latitude, approximately ±85.0511°. Coordinates beyond
 * this range are clamped to keep projected positions finite.
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
 * Returns approximate ground meters per pixel at the latitude and zoom,
 * using the equatorial circumference and cosine latitude correction.
 */
export function metersPerPixel(latitude: number, zoom: number): number {
  const clamped = clamp(latitude, -MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
  return (EARTH_CIRCUMFERENCE_M * Math.cos((clamped * Math.PI) / 180)) / worldSize(zoom);
}

/**
 * The on-screen diameter, in pixels, of a granularity area.
 *
 * `precisionM` is half the rounding cell width on each axis. The circle uses
 * the half-diagonal so it covers the cell corners, approximately in ground
 * meters. This is a visual indication of rounding, not a geodetic error bound.
 */
export function areaDiameterPx(precisionM: number, latitude: number, zoom: number): number {
  return (2 * Math.SQRT2 * precisionM) / metersPerPixel(latitude, zoom);
}

/**
 * Returns the zoom at which the approximate precision circle has the requested
 * pixel diameter. Used to limit camera fitting for coarse locations.
 */
export function zoomForPrecision(
  latitude: number,
  precisionM: number,
  targetDiameterPx: number,
): number {
  const clamped = clamp(latitude, -MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
  const numerator = EARTH_CIRCUMFERENCE_M * Math.cos((clamped * Math.PI) / 180) * targetDiameterPx;
  return Math.log2(numerator / (TILE_SIZE * 2 * Math.SQRT2 * precisionM));
}

/** Returns bounds containing the coordinates, or null if none are usable. */
/** Normalize a camera viewport, including repeated worlds and wrapped bounds. */
export function normalizeBounds(bounds: Bounds): Bounds {
  const width = bounds.east - bounds.west;
  return {
    west: width >= 360 ? -180 : wrapLongitude(bounds.west),
    east: width >= 360 ? 180 : wrapLongitude(bounds.east),
    south: Math.max(-90, bounds.south),
    north: Math.min(90, bounds.north),
  };
}

export function wrapLongitude(longitude: number): number {
  if (longitude >= -180 && longitude < 180) return longitude;
  return ((((longitude + 180) % 360) + 360) % 360) - 180;
}

export function boundsOf(coordinates: readonly LngLat[]): Bounds | null {
  const valid = coordinates.filter(
    ({ latitude, longitude }) => Number.isFinite(latitude) && Number.isFinite(longitude),
  );
  if (valid.length === 0) return null;
  const longitudes = valid.map((point) => wrapLongitude(point.longitude)).sort((a, b) => a - b);
  // Omit the largest empty arc. An east edge beyond 180 tells MapLibre to fit
  // the short arc across the antimeridian rather than the rest of the world.
  let gap = -1;
  let start = 0;
  for (let i = 0; i < longitudes.length; i++) {
    const next = i + 1 === longitudes.length ? longitudes[0] + 360 : longitudes[i + 1];
    if (next - longitudes[i] > gap) {
      gap = next - longitudes[i];
      start = (i + 1) % longitudes.length;
    }
  }
  const west = longitudes[start];
  const end = longitudes[(start + longitudes.length - 1) % longitudes.length];
  return {
    west,
    east: end < west ? end + 360 : end,
    south: Math.min(...valid.map((point) => point.latitude)),
    north: Math.max(...valid.map((point) => point.latitude)),
  };
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

/** Default clustering radius in world pixels, matching the 44px touch target. */
export const CLUSTER_RADIUS_PX = 44;

/**
 * Groups nearby pins using input order to choose cluster seeds. A seed absorbs
 * unconsumed pins within radiusPx; absorbed pins do not expand the search.
 * A spatial grid restricts searches to nearby cells. Results are deterministic
 * for the same ordered input and zoom.
 */
export function clusterPins<T extends Clusterable>(
  pins: readonly T[],
  zoom: number,
  radiusPx: number = CLUSTER_RADIUS_PX,
): MapCluster<T>[] {
  const size = worldSize(zoom);
  const cellSize = Math.max(radiusPx, Number.EPSILON);
  const points = pins.map((pin) => ({
    pin,
    world: project({ ...pin.coordinate, longitude: wrapLongitude(pin.coordinate.longitude) }, zoom),
  }));
  const cells = new Map<string, Set<number>>();
  const key = (x: number, y: number) => `${Math.floor(x / cellSize)},${Math.floor(y / cellSize)}`;
  const keys = points.map(({ world }) =>
    [-size, 0, size].map((offset) => key(world.x + offset, world.y)),
  );
  keys.forEach((pointKeys, index) =>
    pointKeys.forEach((cell) => {
      const members = cells.get(cell) ?? new Set<number>();
      members.add(index);
      cells.set(cell, members);
    }),
  );
  const taken = new Set<number>();
  const clusters: MapCluster<T>[] = [];
  const radiusSquared = radiusPx * radiusPx;
  for (let i = 0; i < points.length; i++) {
    if (taken.has(i)) continue;
    const seed = points[i];
    const nearby = new Set<number>();
    for (const dx of [-cellSize, 0, cellSize]) {
      for (const dy of [-cellSize, 0, cellSize]) {
        for (const index of cells.get(key(seed.world.x + dx, seed.world.y + dy)) ?? [])
          nearby.add(index);
      }
    }
    const members: T[] = [];
    let sumX = 0;
    let sumY = 0;
    // Sort only nearby candidates, preserving the original voyage order.
    for (const index of [...nearby].sort((a, b) => a - b)) {
      const point = points[index];
      const rawDx = point.world.x - seed.world.x;
      const dx = rawDx - Math.round(rawDx / size) * size;
      const dy = point.world.y - seed.world.y;
      if (dx * dx + dy * dy > radiusSquared) continue;
      taken.add(index);
      for (const cell of keys[index]) cells.get(cell)!.delete(index);
      members.push(point.pin);
      sumX += seed.world.x + dx;
      sumY += point.world.y;
    }
    const center = unproject({ x: sumX / members.length, y: sumY / members.length }, zoom);
    clusters.push({
      id: seed.pin.id,
      members,
      coordinate:
        members.length === 1
          ? seed.pin.coordinate
          : { ...center, longitude: wrapLongitude(center.longitude) },
    });
  }

  return clusters;
}

/** Bubble size by count: single digits, tens, or hundreds and above. */
export function clusterTier(count: number): "sm" | "md" | "lg" {
  if (count >= 100) return "lg";
  if (count >= 10) return "md";
  return "sm";
}
