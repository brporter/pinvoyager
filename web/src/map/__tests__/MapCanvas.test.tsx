import { act, cleanup, render } from "@testing-library/react";
import * as maplibregl from "maplibre-gl";
import { afterEach, describe, expect, it, vi } from "vitest";

import { __maps, __resetMaps } from "../../../__mocks__/maplibre-gl";
import { MapCanvas, MapMarker } from "../MapCanvas";

const BOUNDS = { west: 10, south: 20, east: 30, north: 40 };

afterEach(() => {
  cleanup();
  __resetMaps();
  vi.restoreAllMocks();
});

describe("MapCanvas", () => {
  it("uses browser caching and identifies OSM requests without sharing page paths or credentials", async () => {
    render(<MapCanvas label="Test map" />);
    const map = __maps[0];
    const url = "https://tile.openstreetmap.org/2/1/1.png";
    expect(map.options.style).toMatchObject({
      sources: {
        basemap: {
          type: "raster",
          tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
          maxzoom: 19,
          attribution: "© OpenStreetMap contributors",
        },
      },
    });
    const transform = (map.options as unknown as maplibregl.MapOptions).transformRequest!;
    const parameters = await transform(url, "Tile" as maplibregl.ResourceType);

    expect(parameters).toEqual({
      url,
      cache: "default",
      credentials: "same-origin",
      referrerPolicy: "strict-origin",
    });
    expect(map.options.refreshExpiredTiles).toBe(true);

    // Request validates these settings through the platform API without
    // sending traffic to OSM or downloading tiles during automated tests.
    const request = new Request(url, parameters as RequestInit);
    expect(request.cache).toBe("default");
    expect(request.credentials).toBe("same-origin");
    expect(request.referrerPolicy).toBe("strict-origin");
    expect([...request.headers]).toEqual([]);
  });

  it("only renders map children after the initial visible bounds are known", () => {
    const windows: ({ west: number; south: number; east: number; north: number } | null)[] = [];

    render(
      <MapCanvas label="Test map">
        {(view) => {
          windows.push(view.window);
          return null;
        }}
      </MapCanvas>,
    );

    expect(windows.length).toBeGreaterThan(0);
    expect(windows.every((window) => window !== null)).toBe(true);
  });

  it("keeps the current visible bounds when a movement settles at the same bounds", () => {
    const { container } = render(
      <MapCanvas label="Test map">{(view) => <span>{view.window?.west}</span>}</MapCanvas>,
    );
    const map = __maps[0];

    act(() => map.easeTo({}));

    expect(container).toHaveTextContent(String(map.getBounds().getWest()));
  });

  it("fits a changed rectangle on the existing map and removes it on unmount", () => {
    const { rerender, unmount } = render(<MapCanvas label="Test map" />);
    const map = __maps[0];

    expect(map.options.center).toEqual([0, 20]);

    rerender(<MapCanvas label="Test map" fitTo={BOUNDS} maxFitZoom={12} />);

    expect(map.camera.at(-1)).toMatchObject({
      kind: "fitBounds",
      bounds: [
        [10, 20],
        [30, 40],
      ],
      options: { maxZoom: 12 },
    });

    act(() => {
      map.fitBounds(
        [
          [10, 20],
          [30, 40],
        ],
        { maxZoom: 12 },
      );
    });

    unmount();
    expect(map.removed).toBe(true);
  });

  it("does not construct a MapLibre marker without a map context", () => {
    const marker = vi.spyOn(maplibregl, "Marker");

    render(
      <MapMarker coordinate={{ longitude: 1, latitude: 2 }}>
        <button type="button">Pin</button>
      </MapMarker>,
    );

    expect(marker).not.toHaveBeenCalled();
  });
});
