import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { __maps, __resetMaps } from "../../../__mocks__/maplibre-gl";
import PrecisionPreview from "../PrecisionPreview";

afterEach(__resetMaps);

it("loads St. Louis tiles and switches between a position and geographic areas", () => {
  const view = render(<PrecisionPreview precision="exact" />);
  const map = __maps[0];
  expect(map.options.style).toMatchObject({
    sources: {
      basemap: {
        type: "raster",
        tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
      },
    },
  });
  expect((map.getBounds().getWest() + map.getBounds().getEast()) / 2).toBeCloseTo(-90.1994);
  expect((map.getBounds().getSouth() + map.getBounds().getNorth()) / 2).toBeCloseTo(38.627);
  expect(
    screen.getByRole("img", { name: "Example photo location in St. Louis" }),
  ).toBeInTheDocument();
  expect(screen.getByText(/OpenStreetMap/)).toBeInTheDocument();

  for (const precision of ["km1", "km10", "km50"] as const) {
    view.rerender(<PrecisionPreview precision={precision} />);
    expect(
      screen.queryByRole("img", { name: "Example photo location in St. Louis" }),
    ).not.toBeInTheDocument();
    const area = screen.getByRole("img", { name: "Example location area around St. Louis" });
    const diameter = Number.parseFloat(area.style.width);
    expect(diameter).toBeGreaterThan(100);
    expect(diameter).toBeLessThan(150);
    const bounds = map.camera.at(-1)!.bounds as number[][];
    expect((bounds[0][0] + bounds[1][0]) / 2).toBeCloseTo(-90.1994);
    expect((bounds[0][1] + bounds[1][1]) / 2).toBeCloseTo(38.627);
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(
      Number.parseFloat(
        screen.getByRole("img", { name: "Example location area around St. Louis" }).style.width,
      ),
    ).toBeGreaterThan(diameter);
  }
  view.rerender(<PrecisionPreview precision={null} />);
  expect(screen.queryByRole("img", { name: /Example/ })).not.toBeInTheDocument();
  expect(
    screen.getByRole("region", { name: "Location precision example — St. Louis, Missouri" }),
  ).toBeInTheDocument();
});
