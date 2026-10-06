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
      <MapCanvas label="Test map">
        {(view) => <span>{view.window?.west}</span>}
      </MapCanvas>,
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
