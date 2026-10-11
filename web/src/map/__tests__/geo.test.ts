/**
 * The map's arithmetic.
 *
 * These are the calculations that decide what a reader sees: whether two
 * photographs merge into one bubble, how large the circle around a rounded
 * coordinate is, and how far in the camera goes. All three are wrong in ways
 * that look plausible on screen — a circle 30% too small still looks like a
 * circle — so they are checked against figures worked out independently rather
 * than against whatever the implementation currently returns.
 */

import { describe, expect, it } from "vitest";

import {
  CLUSTER_RADIUS_PX,
  MAX_MERCATOR_LATITUDE,
  areaDiameterPx,
  boundsOf,
  clusterPins,
  clusterTier,
  metersPerPixel,
  project,
  unproject,
  zoomForPrecision,
  type Clusterable,
} from "../geo";

/* ==========================================================================
   Projection
   ========================================================================== */

describe("project", () => {
  /** At zoom 0 the whole world is one 512px world, so the null island is at its
   *  center. Every other projected figure is a scaling of this one. */
  it("puts 0°, 0° at the center of the zoom-0 world", () => {
    expect(project({ latitude: 0, longitude: 0 }, 0)).toEqual({ x: 256, y: 256 });
  });

  it("puts the antimeridian at the two edges", () => {
    expect(project({ latitude: 0, longitude: -180 }, 0).x).toBeCloseTo(0, 6);
    expect(project({ latitude: 0, longitude: 180 }, 0).x).toBeCloseTo(512, 6);
  });

  /** The world doubles in each direction per zoom level. Getting this wrong
   *  makes every pixel distance — and therefore all clustering — wrong by a
   *  power of two. */
  it("doubles the world with each zoom level", () => {
    const atFour = project({ latitude: 0, longitude: 90 }, 4);
    const atFive = project({ latitude: 0, longitude: 90 }, 5);

    expect(atFive.x).toBeCloseTo(atFour.x * 2, 6);
    expect(atFive.y).toBeCloseTo(atFour.y * 2, 6);
  });

  it("increases y southward, matching the screen", () => {
    const north = project({ latitude: 40, longitude: 0 }, 8);
    const south = project({ latitude: -40, longitude: 0 }, 8);

    expect(north.y).toBeLessThan(south.y);
  });

  /**
   * The projection sends the poles to infinity. A photograph from a research
   * station is a real photograph, so the latitude is clamped rather than the
   * coordinate rejected — an infinite `y` would make the marker unplaceable and
   * poison the bounds of every other pin in the voyage.
   */
  it("clamps beyond the Mercator limit instead of producing infinity", () => {
    const pole = project({ latitude: 90, longitude: 0 }, 4);
    const limit = project({ latitude: MAX_MERCATOR_LATITUDE, longitude: 0 }, 4);

    expect(Number.isFinite(pole.y)).toBe(true);
    expect(pole.y).toBeCloseTo(limit.y, 6);
  });
});

describe("unproject", () => {
  it("is the inverse of project", () => {
    for (const coordinate of [
      { latitude: 0, longitude: 0 },
      { latitude: 43.06, longitude: 141.35 },
      { latitude: -33.87, longitude: 151.21 },
      { latitude: 64.14, longitude: -21.94 },
    ]) {
      const roundTrip = unproject(project(coordinate, 12), 12);

      expect(roundTrip.latitude).toBeCloseTo(coordinate.latitude, 9);
      expect(roundTrip.longitude).toBeCloseTo(coordinate.longitude, 9);
    }
  });
});

/* ==========================================================================
   Ground resolution
   ========================================================================== */

describe("metersPerPixel", () => {
  /**
   * 40075016.686 m / 512 px. This is the published Web Mercator figure, and
   * quoting it here is the point — it is not derived from the implementation.
   */
  it("is 78271.52 m/px at the equator at zoom 0", () => {
    expect(metersPerPixel(0, 0)).toBeCloseTo(78271.51696, 4);
  });

  it("halves with each zoom level", () => {
    expect(metersPerPixel(0, 10)).toBeCloseTo(metersPerPixel(0, 9) / 2, 9);
  });

  /**
   * The latitude correction, which is the reason this function takes a latitude
   * at all. Mercator stretches distance by 1/cos(latitude), so a pixel at 60°
   * covers half the ground a pixel at the equator does. A single constant would
   * draw northern granularity circles at roughly twice the radius they should
   * be, and a voyage in Reykjavík would look far vaguer than the same voyage in
   * Singapore.
   */
  it("shrinks with latitude by cos(latitude)", () => {
    expect(metersPerPixel(60, 10)).toBeCloseTo(metersPerPixel(0, 10) * Math.cos(Math.PI / 3), 6);
  });

  it("is symmetric about the equator", () => {
    expect(metersPerPixel(-45, 8)).toBeCloseTo(metersPerPixel(45, 8), 9);
  });
});

describe("areaDiameterPx", () => {
  /**
   * A ~1 km voyage means "within one kilometer of this point", so the circle is
   * two kilometers across. Drawing it at one kilometer across would understate
   * the imprecision by half, which is the direction that matters — it would
   * claim more accuracy than the data has.
   */
  it("draws a circle of twice the precision radius", () => {
    const zoom = 12;
    const latitude = 0;
    const meters = metersPerPixel(latitude, zoom);

    expect(areaDiameterPx(1000, latitude, zoom)).toBeCloseTo(Math.hypot(2000, 2000) / meters, 6);
  });

  it("grows as the camera zooms in", () => {
    expect(areaDiameterPx(1000, 45, 14)).toBeGreaterThan(areaDiameterPx(1000, 45, 12));
  });

  /** ~50 km must be visibly bigger than ~1 km at the same zoom, or the setting
   *  the user chose is invisible to a reader. */
  it("separates the granularity settings at one zoom", () => {
    const at = (precision: number) => areaDiameterPx(precision, 43, 11);

    expect(at(50_000)).toBeGreaterThan(at(10_000));
    expect(at(10_000)).toBeGreaterThan(at(1_000));
  });
});

describe("zoomForPrecision", () => {
  /**
   * The two are inverses, and that is what makes the camera cap meaningful:
   * fitting to `zoomForPrecision(lat, p, D)` puts a circle of exactly `D`
   * pixels on screen. If they disagreed, a single-photograph voyage at ~50 km
   * would still zoom to street level and make a claim its coordinates cannot
   * support.
   */
  it("is the zoom at which the area is the requested size", () => {
    for (const [latitude, precision, target] of [
      [0, 1_000, 320],
      [43.06, 10_000, 320],
      [-33.87, 50_000, 240],
      [64.14, 1_000, 120],
    ] as const) {
      const zoom = zoomForPrecision(latitude, precision, target);

      expect(areaDiameterPx(precision, latitude, zoom)).toBeCloseTo(target, 6);
    }
  });

  it("allows a closer camera for a more precise voyage", () => {
    expect(zoomForPrecision(43, 1_000, 320)).toBeGreaterThan(zoomForPrecision(43, 50_000, 320));
  });
});

/* ==========================================================================
   Bounds
   ========================================================================== */

describe("boundsOf", () => {
  it("has no answer for an empty list", () => {
    expect(boundsOf([])).toBeNull();
  });

  it("returns the extremes", () => {
    expect(
      boundsOf([
        { latitude: 43.06, longitude: 141.35 },
        { latitude: 43.21, longitude: 141.5 },
        { latitude: 42.9, longitude: 141.1 },
      ]),
    ).toEqual({ west: 141.1, south: 42.9, east: 141.5, north: 43.21 });
  });

  it("gives one coordinate a rectangle with no area rather than nothing", () => {
    expect(boundsOf([{ latitude: 10, longitude: 20 }])).toEqual({
      west: 20,
      south: 10,
      east: 20,
      north: 10,
    });
  });

  /**
   * A malformed payload containing a single NaN in the list would
   * otherwise make every comparison false and leave the camera unable to fit
   * anything at all. Every other pin in the voyage would disappear along with
   * the bad one.
   */
  it("ignores a coordinate that is not a finite number", () => {
    expect(
      boundsOf([
        { latitude: 43.06, longitude: 141.35 },
        { latitude: Number.NaN, longitude: 141.5 },
        { latitude: 43.21, longitude: Number.POSITIVE_INFINITY },
      ]),
    ).toEqual({ west: 141.35, south: 43.06, east: 141.35, north: 43.06 });
  });

  it("has no answer when every coordinate is unusable", () => {
    expect(boundsOf([{ latitude: Number.NaN, longitude: Number.NaN }])).toBeNull();
  });
});

/* ==========================================================================
   Clustering
   ========================================================================== */

function pin(id: string, latitude: number, longitude: number): Clusterable {
  return { id, coordinate: { latitude, longitude } };
}

describe("clusterPins", () => {
  it("leaves a pin alone when nothing is near it", () => {
    const clusters = clusterPins([pin("a", 0, 0), pin("b", 0, 40)], 10);

    expect(clusters).toHaveLength(2);
    expect(clusters.map((cluster) => cluster.members.length)).toEqual([1, 1]);
  });

  /**
   * At zoom 10 one degree of longitude is about 728 world pixels, so 0.01° is
   * roughly 7px — well inside the 44px merge distance, and two markers that
   * close cannot be tapped apart.
   */
  it("merges pins that would overlap on screen", () => {
    const clusters = clusterPins([pin("a", 0, 0), pin("b", 0, 0.01)], 10);

    expect(clusters).toHaveLength(1);
    expect(clusters[0].members.map((member) => member.id)).toEqual(["a", "b"]);
  });

  /** The same two pins, four zoom levels closer: now about 116px apart, so they
   *  separate. This is target 7 — clusters expand as the camera zooms in. */
  it("separates the same pins once the camera is close enough", () => {
    const pins = [pin("a", 0, 0), pin("b", 0, 0.01)];

    expect(clusterPins(pins, 10)).toHaveLength(1);
    expect(clusterPins(pins, 14)).toHaveLength(2);
  });

  /**
   * The seed-only rule. At zoom 10 the merge distance is about 0.06°, so `b` is
   * within reach of `a` and `c` is within reach of `b` but not of `a`. If an
   * absorbed pin went on to absorb its own neighbors, a line of photographs
   * taken along a road would chain into one bubble spanning the whole route,
   * and that bubble would be drawn somewhere none of them were taken.
   */
  it("does not let an absorbed pin drag in its own neighbors", () => {
    const clusters = clusterPins([pin("a", 0, 0), pin("b", 0, 0.025), pin("c", 0, 0.05)], 10);

    expect(clusters).toHaveLength(2);
    expect(clusters[0].members.map((member) => member.id)).toEqual(["a", "b"]);
    expect(clusters[1].members.map((member) => member.id)).toEqual(["c"]);
  });

  it("skips a pin already absorbed by an earlier cluster", () => {
    const clusters = clusterPins(
      [pin("seed", 0, 0), pin("separate", 0, 0.1), pin("absorbed", 0, 0.01)],
      10,
    );

    expect(clusters).toHaveLength(2);
    expect(clusters[0].members.map(({ id }) => id)).toEqual(["seed", "absorbed"]);
    expect(clusters[1].members.map(({ id }) => id)).toEqual(["separate"]);
  });

  it("keeps every pin, in exactly one cluster", () => {
    const pins = Array.from({ length: 40 }, (_, index) =>
      pin(`p${index}`, 43 + index * 0.002, 141 + index * 0.003),
    );

    const clusters = clusterPins(pins, 11);
    const ids = clusters.flatMap((cluster) => cluster.members.map((member) => member.id));

    expect(ids).toHaveLength(pins.length);
    expect(new Set(ids).size).toBe(pins.length);
  });

  /** A single-member cluster stands exactly on its photograph. Nudging it to a
   *  computed centroid would move every unclustered pin off its own
   *  coordinate. */
  it("leaves a lone pin on its own coordinate", () => {
    const [cluster] = clusterPins([pin("a", 43.06, 141.35)], 14);

    expect(cluster.coordinate).toEqual({ latitude: 43.06, longitude: 141.35 });
  });

  /**
   * The bubble sits between its members rather than on the first of them.
   * Averaged in world pixels, not in degrees: Mercator is not linear in
   * latitude, so averaging degrees puts the bubble visibly off-center at high
   * latitudes. At 60° the two are far enough apart to tell.
   */
  it("places a bubble at the middle of its members, measured on the map", () => {
    const [cluster] = clusterPins([pin("a", 60, 0), pin("b", 60.02, 0)], 8);

    expect(cluster.members).toHaveLength(2);

    const middleInWorldPixels = unproject(
      {
        x:
          (project({ latitude: 60, longitude: 0 }, 8).x +
            project({ latitude: 60.02, longitude: 0 }, 8).x) /
          2,
        y:
          (project({ latitude: 60, longitude: 0 }, 8).y +
            project({ latitude: 60.02, longitude: 0 }, 8).y) /
          2,
      },
      8,
    );

    expect(cluster.coordinate.latitude).toBeCloseTo(middleInWorldPixels.latitude, 9);
    // And it is not the naive average of the two latitudes.
    expect(cluster.coordinate.latitude).not.toBeCloseTo(60.01, 9);
  });

  /** The id is a React key. A cluster whose membership has not changed must
   *  keep it, or the marker is destroyed and rebuilt on every zoom tick and any
   *  focus inside it is lost. */
  it("identifies a cluster by its seed", () => {
    const pins = [pin("a", 0, 0), pin("b", 0, 0.01)];

    expect(clusterPins(pins, 10)[0].id).toBe("a");
    expect(clusterPins(pins, 10.5)[0].id).toBe("a");
  });

  it("has nothing to do with an empty voyage", () => {
    expect(clusterPins([], 10)).toEqual([]);
  });

  /** Pins at exactly the same rounded coordinate — which is what a coarse
   *  granularity produces for a whole town — must not be drawn as a stack of
   *  identical markers, because a stack of pins is one pin to the reader. */
  it("merges pins that round to the same coordinate", () => {
    const clusters = clusterPins(
      [pin("a", 43.05, 141.35), pin("b", 43.05, 141.35), pin("c", 43.05, 141.35)],
      16,
    );

    expect(clusters).toHaveLength(1);
    expect(clusters[0].members).toHaveLength(3);
  });

  it("uses the touch target as its default merge distance", () => {
    expect(CLUSTER_RADIUS_PX).toBe(44);
  });
});

describe("clusterTier", () => {
  /** The design draws the three tiers with 7, 24 and 156. The purpose is that a
   *  glance separates "a handful" from "a lot" without reading the number. */
  it("gives the design's own examples the sizes they are drawn at", () => {
    expect(clusterTier(7)).toBe("sm");
    expect(clusterTier(24)).toBe("md");
    expect(clusterTier(156)).toBe("lg");
  });

  it("changes tier at ten and at a hundred", () => {
    expect(clusterTier(9)).toBe("sm");
    expect(clusterTier(10)).toBe("md");
    expect(clusterTier(99)).toBe("md");
    expect(clusterTier(100)).toBe("lg");
  });
});

it("fits the short arc across the antimeridian", () => {
  const bounds = boundsOf([
    { latitude: -15, longitude: 179 },
    { latitude: -17, longitude: -179 },
  ])!;
  expect(bounds.east - bounds.west).toBe(2);
});

it("clusters neighboring pins across the antimeridian", () => {
  const pins = [
    { id: "east", coordinate: { latitude: 0, longitude: 179.99 } },
    { id: "west", coordinate: { latitude: 0, longitude: -179.99 } },
  ];
  const clusters = clusterPins(pins, 8);
  expect(clusters).toHaveLength(1);
  expect(Math.abs(clusters[0].coordinate.longitude)).toBeCloseTo(180);
});
it("shows the diagonal extent of a rounding cell", () => {
  const diameterM = areaDiameterPx(500, 0, 12) * metersPerPixel(0, 12);
  expect(diameterM).toBeCloseTo(Math.hypot(1000, 1000));
});
