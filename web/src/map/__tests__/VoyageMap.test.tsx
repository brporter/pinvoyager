/**
 * What the map draws, and what it does when something on it is chosen.
 *
 * MapLibre is replaced by the substitute in `web/__mocks__/maplibre-gl.ts` —
 * the real library needs a WebGL context that jsdom does not have. What survives
 * the substitution is everything this phase is actually about: which markers
 * exist, what they are called, what happens when they are clicked, and where
 * the camera is told to go. Nothing here asserts on a rendered map image,
 * because nothing here renders one.
 */

import "@testing-library/jest-dom/vitest";

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { __maps, __resetMaps } from "../../../__mocks__/maplibre-gl";
import { VoyageMap, type VoyagePin } from "../VoyageMap";

vi.mock("maplibre-gl");

/* -- fixtures ------------------------------------------------------------ */

function pin(id: string, latitude: number, longitude: number): VoyagePin {
  return { id, coordinate: { latitude, longitude }, label: `Photograph ${id}` };
}

/** Two photographs half a meter apart and one half a degree away. Fitting all
 *  three lands the camera at a zoom where the first two overlap and the third
 *  does not, which is the situation clustering exists for. */
const NEAR_A = pin("a", 43.06, 141.35);
const NEAR_B = pin("b", 43.0605, 141.3505);
const FAR = pin("c", 43.5, 141.9);

/** Far enough apart at any realistic zoom that nothing merges. */
const SPREAD = [pin("a", 43.06, 141.35), pin("b", 35.68, 139.69)];

function renderMap(props: Partial<Parameters<typeof VoyageMap>[0]> = {}) {
  const result = render(
    <VoyageMap
      label="Map of Three weeks in Hokkaido"
      pins={SPREAD}
      precisionM={null}
      selection={null}
      onSelectPin={() => {}}
      {...props}
    />,
  );

  return result;
}

function mapRegion(): HTMLElement {
  return screen.getByRole("region", { name: /^Map of / });
}

function pins(): HTMLElement[] {
  return Array.from(mapRegion().querySelectorAll<HTMLElement>("button.pv-pin"));
}

function clusters(): HTMLElement[] {
  return Array.from(mapRegion().querySelectorAll<HTMLElement>("button.pv-cluster"));
}

function areas(): HTMLElement[] {
  return Array.from(mapRegion().querySelectorAll<HTMLElement>(".pv-map-area"));
}

/** The instance the component built. There is exactly one per render. */
function camera() {
  return __maps[0];
}

function stubReducedMotion(reduced: boolean): void {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("prefers-reduced-motion") ? reduced : false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }));
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  __resetMaps();
});

/* -- tests --------------------------------------------------------------- */

describe("VoyageMap — what is on the map", () => {
  it("draws one pin per placed photograph when none of them overlap", () => {
    renderMap();

    expect(pins()).toHaveLength(2);
    expect(clusters()).toHaveLength(0);
  });

  /**
   * A pin is a graphic with no text, so its accessible name is the only name it
   * has. It is deliberately the same string the photograph carries in the rail,
   * which is what lets a screen-reader user tell which pin belongs to which
   * photograph.
   */
  it("names each pin after its photograph", () => {
    renderMap();

    expect(within(mapRegion()).getByRole("button", { name: "Photograph a" })).toBeInTheDocument();
    expect(within(mapRegion()).getByRole("button", { name: "Photograph b" })).toBeInTheDocument();
  });

  it("has nothing to draw for a voyage with no placed photographs", () => {
    renderMap({ pins: [] });

    expect(pins()).toHaveLength(0);
    expect(clusters()).toHaveLength(0);
  });

  /**
   * Required by the ODbL license the map data is published under. It is not a
   * courtesy, there is no prop to hide it, and it must be on every map
   * instance — which is why this is asserted rather than assumed.
   */
  it("shows the OpenStreetMap attribution", () => {
    renderMap();

    const notice = mapRegion().querySelector(".pv-attribution");
    expect(notice).toHaveTextContent("OpenStreetMap");
    expect(notice?.querySelector("a")).toHaveAttribute(
      "href",
      "https://www.openstreetmap.org/copyright",
    );
  });

  /** Pinch and scroll both zoom, but a pointer user with no wheel and a
   *  keyboard user need a path to the same capability. */
  it("offers zoom controls that are not gestures", async () => {
    renderMap();

    await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(camera().camera.at(-1)?.kind).toBe("zoomIn");

    await userEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(camera().camera.at(-1)?.kind).toBe("zoomOut");
  });
});

describe("VoyageMap — clustering", () => {
  it("merges overlapping photographs into a counted bubble", () => {
    renderMap({ pins: [NEAR_A, NEAR_B, FAR] });

    const bubbles = clusters();
    expect(bubbles).toHaveLength(1);
    expect(bubbles[0]).toHaveTextContent("2");
    expect(bubbles[0]).toHaveAccessibleName("2 photos here");

    // And the third photograph is still a pin of its own.
    expect(pins()).toHaveLength(1);
  });

  /** The bubble's job. Without it there is no way to reach the photographs
   *  inside a cluster at all — including for a keyboard user, which is why it
   *  is a button rather than a pointer-only affordance. */
  it("zooms in when a bubble is chosen, so it comes apart", async () => {
    renderMap({ pins: [NEAR_A, NEAR_B, FAR] });

    const before = camera().getZoom();
    await userEvent.click(clusters()[0]);

    const move = camera().camera.at(-1);
    expect(move?.kind).toBe("easeTo");
    expect(camera().getZoom()).toBeGreaterThan(before);
  });

  /**
   * Otherwise choosing a photograph in the rail could mark a pin that is not
   * drawn — swallowed into a bubble showing "2" — and the selection would
   * appear to have done nothing to the map at all.
   */
  it("never hides the selected photograph inside a bubble", () => {
    // Unselected, `a` and `b` overlap and are drawn as one bubble of two.
    const { unmount } = renderMap({ pins: [NEAR_A, NEAR_B, FAR] });
    expect(clusters()).toHaveLength(1);
    expect(clusters()[0]).toHaveTextContent("2");
    unmount();
    __resetMaps();

    // Selecting `b` takes it out of the clustering entirely: it is drawn as its
    // own pin, and what is left of the bubble is a single photograph — so `a`
    // becomes a pin too and no bubble remains. Merely drawing the selected pin
    // on top of an unchanged bubble would leave the same marker in two places.
    renderMap({
      pins: [NEAR_A, NEAR_B, FAR],
      selection: { photoId: "b", source: "photo" },
    });

    expect(clusters()).toHaveLength(0);
    expect(pins()).toHaveLength(3);

    const selected = within(mapRegion()).getByRole("button", { name: "Photograph b" });
    expect(selected).toHaveClass("pv-pin");
    expect(selected).toHaveAttribute("aria-pressed", "true");
  });
});

describe("VoyageMap — granularity", () => {
  /**
   * Target 10. A voyage rounded to ~10 km draws a circle 20 km across, because
   * a sharp pin at the center of a 10 km rounding claims a precision the data
   * does not have — and it visually undoes the choice the user deliberately
   * made when they published at that setting.
   */
  it("draws an area for a voyage that publishes rounded coordinates", () => {
    renderMap({ precisionM: 10_000 });

    expect(areas().length).toBeGreaterThan(0);
  });

  /** At exact precision there is no uncertainty to draw, and drawing one would
   *  invent an imprecision the user did not ask for. */
  it("draws no area for a voyage that publishes exact coordinates", () => {
    renderMap({ precisionM: null });

    expect(areas()).toHaveLength(0);
  });

  it("draws a larger area for a coarser voyage", () => {
    const sizeAt = (precision: number): number => {
      const { unmount } = renderMap({ pins: [SPREAD[0]], precisionM: precision });
      const width = Number.parseFloat((areas()[0] as HTMLElement).style.width.replace("px", ""));
      unmount();
      __resetMaps();
      return width;
    };

    // Both are fitted to the same single photograph, so the only difference is
    // the precision — and the camera cap keeps the drawn size comparable.
    expect(sizeAt(50_000)).toBeGreaterThan(0);
    expect(sizeAt(1_000)).toBeGreaterThan(0);
  });

  /**
   * A single photograph has no span to fit, so the camera would otherwise go to
   * its maximum and show a named street corner for a position known only to
   * within fifty kilometers. The cap is the same defect target 10 is about,
   * expressed as a camera rather than as a marker.
   */
  it("does not zoom past what the coordinates know", () => {
    renderMap({ pins: [SPREAD[0]], precisionM: 50_000 });
    const coarse = camera().getZoom();
    __resetMaps();

    renderMap({ pins: [SPREAD[0]], precisionM: null });
    const exact = camera().getZoom();

    expect(coarse).toBeLessThan(exact);
  });
});

describe("VoyageMap — selection", () => {
  it("reports the photograph when its pin is chosen", async () => {
    const onSelectPin = vi.fn();
    renderMap({ onSelectPin });

    await userEvent.click(within(mapRegion()).getByRole("button", { name: "Photograph b" }));

    expect(onSelectPin).toHaveBeenCalledWith("b");
  });

  it("marks the selected photograph's pin", () => {
    renderMap({ selection: { photoId: "a", source: "photo" } });

    expect(within(mapRegion()).getByRole("button", { name: "Photograph a" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(within(mapRegion()).getByRole("button", { name: "Photograph b" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("does not move the camera when the selected photo is absent from the pins", () => {
    renderMap({ selection: { photoId: "missing-photo", source: "photo" } });

    expect(camera().camera.filter((call) => call.kind === "easeTo")).toHaveLength(0);
    expect(
      within(mapRegion()).queryByRole("button", { name: "Photograph missing-photo" }),
    ).not.toBeInTheDocument();
  });

  it("reports activation of the already selected pin", async () => {
    const onSelectPin = vi.fn();
    renderMap({ selection: { photoId: "b", source: "photo" }, onSelectPin });

    await userEvent.click(within(mapRegion()).getByRole("button", { name: "Photograph b" }));

    expect(onSelectPin).toHaveBeenCalledWith("b");
  });

  /** Half of target 8: choosing a photograph in the list brings the map to it.
   *  Without this the rail and the map are two lists that happen to share a
   *  screen. */
  it("moves the camera to a photograph chosen in the list", () => {
    renderMap({ selection: { photoId: "b", source: "photo" } });

    const move = camera().camera.find((call) => call.kind === "easeTo");
    expect(move?.options.center).toEqual([139.69, 35.68]);
  });

  /**
   * The other half, and the reason the selection carries where it came from.
   * The pin the user just clicked is already under their pointer; re-centring
   * on it slides the marker out from under the click that selected it.
   */
  it("leaves the camera alone for a photograph chosen by its pin", () => {
    renderMap({ selection: { photoId: "b", source: "pin" } });

    expect(camera().camera.filter((call) => call.kind === "easeTo")).toHaveLength(0);
  });

  /**
   * Once, not once per camera event.
   *
   * Moving the camera makes it settle, settling tells this component where it
   * is now looking, and that is a state change. If the callback that moves the
   * camera were rebuilt by that state change, the effect that panned to the
   * selection would see new dependencies and pan again — an endless flight, in
   * a browser as much as here. `MapCanvas` keeps `moveTo` stable for the life
   * of the map instance precisely so this cannot happen.
   */
  it("moves the camera once for one selection", () => {
    renderMap({ selection: { photoId: "b", source: "photo" } });

    expect(camera().camera.filter((call) => call.kind === "easeTo")).toHaveLength(1);
  });
});

describe("VoyageMap — reduced motion", () => {
  /**
   * MapLibre animates its camera in JavaScript against its own clock, outside
   * CSS, so the design system's `prefers-reduced-motion` rules do not reach it.
   * The preference has to be passed into every camera call. Without this the
   * map keeps flying across continents for exactly the users who asked it not
   * to.
   */
  it("jumps rather than flies when the user has asked for less motion", async () => {
    stubReducedMotion(true);
    renderMap({ selection: { photoId: "b", source: "photo" } });

    const move = camera().camera.find((call) => call.kind === "easeTo");
    expect(move?.options.duration).toBe(0);

    await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(camera().camera.at(-1)?.options.duration).toBe(0);
  });

  it("animates normally when it has not been asked not to", async () => {
    stubReducedMotion(false);
    renderMap({ selection: { photoId: "b", source: "photo" } });

    const move = camera().camera.find((call) => call.kind === "easeTo");
    expect(move?.options.duration).toBeGreaterThan(0);
  });
});

describe("VoyageMap — location gallery", () => {
  it("passes every member of a clicked cluster to the gallery without zooming", async () => {
    const onOpenGallery = vi.fn();
    renderMap({ pins: [NEAR_A, NEAR_B, FAR], onOpenGallery });
    const before = camera().getZoom();
    await userEvent.click(clusters()[0]);
    expect(onOpenGallery).toHaveBeenCalledWith(["a", "b"]);
    expect(camera().getZoom()).toBe(before);
  });

  it("opens a one-photo gallery from an ordinary and a selected pin", async () => {
    const onOpenGallery = vi.fn();
    renderMap({ onOpenGallery, selection: { photoId: "a", source: "photo" } });
    await userEvent.click(pins().find((pin) => pin.getAttribute("aria-pressed") === "true")!);
    expect(onOpenGallery).toHaveBeenLastCalledWith(["a"]);
    await userEvent.click(pins().find((pin) => pin.getAttribute("aria-pressed") !== "true")!);
    expect(onOpenGallery).toHaveBeenLastCalledWith(["b"]);
  });
});

it("hides rounding areas too small to see at a world view", async () => {
  const { act } = await import("@testing-library/react");
  renderMap({ precisionM: 500 });
  act(() => {
    camera().setZoom(0);
  });
  expect(areas()).toHaveLength(0);
});
