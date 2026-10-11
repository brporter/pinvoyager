/** Compare our geometry with the installed MapLibre implementation, without WebGL. */
import { expect, it, vi } from "vitest";
import { normalizeBounds, project } from "../geo";

it("uses MapLibre's camera scale and Mercator coordinates", async () => {
  const { MercatorTransform } = await vi.importActual<{
    MercatorTransform: new () => { tileSize: number };
  }>("../../../node_modules/maplibre-gl/src/geo/projection/mercator_transform.ts");
  const { MercatorCoordinate } = await vi.importActual<typeof import("maplibre-gl")>("maplibre-gl");
  const camera = new MercatorTransform();
  for (const zoom of [0, 8, 15]) {
    for (const [longitude, latitude] of [
      [0, 0],
      [141.35, 43.06],
      [-179.9, -60],
    ]) {
      const real = MercatorCoordinate.fromLngLat({ lng: longitude, lat: latitude });
      const ours = project({ longitude, latitude }, zoom);
      expect(ours.x).toBeCloseTo(real.x * camera.tileSize * 2 ** zoom, 6);
      expect(ours.y).toBeCloseTo(real.y * camera.tileSize * 2 ** zoom, 6);
    }
  }
});

it("normalizes repeated-world viewports without losing a full world", () => {
  expect(normalizeBounds({ west: -220, east: 220, south: -95, north: 95 })).toEqual({
    west: -180,
    east: 180,
    south: -90,
    north: 90,
  });
  expect(normalizeBounds({ west: 170, east: 190, south: -20, north: 20 })).toEqual({
    west: 170,
    east: -170,
    south: -20,
    north: 20,
  });
  expect(normalizeBounds({ west: 530, east: 550, south: -20, north: 20 })).toEqual({
    west: 170,
    east: -170,
    south: -20,
    north: 20,
  });
});
