/**
 * MapLibre test double for jsdom, installed by vi.mock("maplibre-gl").
 *
 * Creates DOM markers and records camera instructions. Camera fitting uses an
 * assumed 800×600 viewport because jsdom has no layout. No tiles are requested
 * and nothing is rendered or animated. Load callbacks fire synchronously.
 * Tests can inspect instances and reset their bookkeeping with __resetMaps.
 */

type Listener = (...args: unknown[]) => void;

export interface FakeCameraCall {
  kind: "easeTo" | "fitBounds" | "zoomIn" | "zoomOut";
  /** Whatever the caller passed. `duration: 0` is how reduced motion arrives. */
  options: Record<string, unknown>;
  /** Present for `fitBounds`. */
  bounds?: unknown;
}

export interface FakeMapOptions {
  container: HTMLElement;
  zoom?: number;
  bounds?: unknown;
  fitBoundsOptions?: Record<string, unknown>;
  [key: string]: unknown;
}

/**
 * Every map built during the current test, oldest first.
 *
 * Exported so a test can assert on the camera without reaching through the
 * component tree. Cleared by {@link __resetMaps}, which the test file's
 * `afterEach` should call — otherwise one test's map is visible to the next.
 */
export const __maps: FakeMap[] = [];

export function __resetMaps(): void {
  __maps.length = 0;
}

/** The viewport this fake map pretends to have. jsdom performs no layout, so
 *  every element measures zero and a real figure has to be assumed. */
const ASSUMED_VIEWPORT = { width: 800, height: 600 };

/** Web Mercator northing at zoom 0, in the 0–512 range. Written out rather than
 *  imported from `src/map/geo.ts` on purpose: a test double that shares the
 *  implementation's arithmetic agrees with it even when the arithmetic is
 *  wrong. */
function mercatorY(latitude: number): number {
  const sin = Math.sin((latitude * Math.PI) / 180);
  return (0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * 512;
}

type BoundsPair = [[number, number], [number, number]];

function centerOf(bounds: BoundsPair): [number, number] {
  const [[west, south], [east, north]] = bounds;
  return [(west + east) / 2, (south + north) / 2];
}

/**
 * The zoom at which a rectangle fits the viewport.
 *
 * A degenerate rectangle — one point, which is a voyage with a single place in
 * it — has no span to fit, so it takes `maxZoom`. That is what the real library
 * does too, and it is why the application passes a `maxZoom` derived from the
 * voyage's precision.
 */
function zoomForBounds(bounds: BoundsPair, options: Record<string, unknown>): number {
  const padding = typeof options.padding === "number" ? options.padding : 0;
  const maxZoom = typeof options.maxZoom === "number" ? options.maxZoom : 20;

  const [[west, south], [east, north]] = bounds;
  const width = Math.max(1, ASSUMED_VIEWPORT.width - 2 * padding);
  const height = Math.max(1, ASSUMED_VIEWPORT.height - 2 * padding);

  const spanX = (Math.abs(east - west) / 360) * 512;
  const spanY = Math.abs(mercatorY(north) - mercatorY(south));

  const zoomX = spanX === 0 ? Infinity : Math.log2(width / spanX);
  const zoomY = spanY === 0 ? Infinity : Math.log2(height / spanY);

  return Math.max(0, Math.min(maxZoom, zoomX, zoomY));
}

export class FakeMap {
  readonly options: FakeMapOptions;
  readonly container: HTMLElement;

  /** Where markers are appended. The real library has one of these too, and
   *  putting markers anywhere else would leave them outside the map's own
   *  accessible region and findable by no realistic query. */
  readonly markerContainer: HTMLElement;

  /** Every camera instruction, in order. */
  readonly camera: FakeCameraCall[] = [];

  removed = false;

  private zoom: number;
  private center: [number, number] = [0, 0];
  private readonly listeners = new globalThis.Map<string, Listener[]>();

  constructor(options: FakeMapOptions) {
    this.options = options;
    this.container = options.container;

    // The application opens the camera on the voyage through the constructor
    // rather than by flying there, so the fitted zoom has to be worked out
    // here as well as in `fitBounds`.
    if (options.bounds !== undefined) {
      this.zoom = zoomForBounds(options.bounds as BoundsPair, options.fitBoundsOptions ?? {});
      this.center = centerOf(options.bounds as BoundsPair);
    } else {
      this.zoom = typeof options.zoom === "number" ? options.zoom : 4;
      if (Array.isArray(options.center)) this.center = options.center as [number, number];
    }

    this.markerContainer = document.createElement("div");
    this.markerContainer.className = "maplibregl-canvas-container";
    this.container.appendChild(this.markerContainer);

    __maps.push(this);
  }

  /** Rotation is switched off by the application; the object has to exist for
   *  that call not to throw. */
  readonly touchZoomRotate = {
    disableRotation: () => {},
    enableRotation: () => {},
  };

  on(event: string, listener: Listener): this {
    // See the note at the top: this map is loaded the moment it exists.
    if (event === "load") {
      listener();
      return this;
    }

    const existing = this.listeners.get(event) ?? [];
    existing.push(listener);
    this.listeners.set(event, existing);
    return this;
  }

  off(event: string, listener: Listener): this {
    const existing = this.listeners.get(event) ?? [];
    this.listeners.set(
      event,
      existing.filter((candidate) => candidate !== listener),
    );
    return this;
  }

  getCanvasContainer(): HTMLElement {
    return this.markerContainer;
  }

  getZoom(): number {
    return this.zoom;
  }

  /**
   * A rectangle around the center.
   *
   * jsdom performs no layout, so there is no viewport to derive one from. The
   * assumed 800x600 is converted to degrees at the current zoom, which is
   * enough for a test to assert that the discovery map asked the server about
   * the area it is looking at rather than about some other one.
   */
  getBounds() {
    const scale = 512 * 2 ** this.zoom;
    const halfLng = (ASSUMED_VIEWPORT.width / 2 / scale) * 360;
    const halfLat = (ASSUMED_VIEWPORT.height / 2 / scale) * 360;

    const [centerLng, centerLat] = this.center;

    return {
      getWest: () => Math.max(-180, centerLng - halfLng),
      getEast: () => Math.min(180, centerLng + halfLng),
      getSouth: () => Math.max(-85, centerLat - halfLat),
      getNorth: () => Math.min(85, centerLat + halfLat),
    };
  }

  /** Moves the camera and tells anything listening, which is what makes
   *  clusters re-form in a test the way they would in a browser. */
  setZoom(zoom: number): this {
    this.zoom = zoom;
    this.fire("zoom");
    return this;
  }

  easeTo(options: Record<string, unknown>): this {
    this.camera.push({ kind: "easeTo", options });
    if (Array.isArray(options.center)) this.center = options.center as [number, number];
    if (typeof options.zoom === "number") this.setZoom(options.zoom);
    this.fire("moveend");
    return this;
  }

  fitBounds(bounds: unknown, options: Record<string, unknown> = {}): this {
    this.camera.push({ kind: "fitBounds", options, bounds });
    this.center = centerOf(bounds as BoundsPair);
    this.setZoom(zoomForBounds(bounds as BoundsPair, options));
    this.fire("moveend");
    return this;
  }

  zoomIn(options: Record<string, unknown> = {}): this {
    this.camera.push({ kind: "zoomIn", options });
    this.setZoom(this.zoom + 1);
    return this;
  }

  zoomOut(options: Record<string, unknown> = {}): this {
    this.camera.push({ kind: "zoomOut", options });
    this.setZoom(this.zoom - 1);
    return this;
  }

  remove(): void {
    this.removed = true;
    this.markerContainer.remove();
    this.listeners.clear();
  }

  private fire(event: string): void {
    for (const listener of this.listeners.get(event) ?? []) listener();
  }
}

export interface FakeMarkerOptions {
  element?: HTMLElement;
  anchor?: string;
}

export class FakeMarker {
  readonly element: HTMLElement;
  lngLat: [number, number] | null = null;

  constructor(options: FakeMarkerOptions = {}) {
    this.element = options.element ?? document.createElement("div");
  }

  setLngLat(value: [number, number]): this {
    this.lngLat = value;
    // Written onto the element so a test can assert where a marker was placed
    // without holding a reference to the marker object. The real library
    // expresses the same thing as a CSS transform, which is unreadable in a
    // renderer with no layout.
    this.element.dataset.lng = String(value[0]);
    this.element.dataset.lat = String(value[1]);
    return this;
  }

  addTo(map: FakeMap): this {
    map.markerContainer.appendChild(this.element);
    return this;
  }

  remove(): this {
    this.element.remove();
    return this;
  }
}

export { FakeMap as Map, FakeMarker as Marker };

export function setMaxParallelImageRequests(_count: number): void {
  // Nothing is downloaded here.
}
