/**
 * The world map on the discovery screen.
 *
 * The thing that distinguishes it from the voyage map is where the grouping
 * happens: the server does it, so this component's job is to ask about the
 * right rectangle and to draw what comes back. Both halves are checked here —
 * what it asks for, and what it does with the answer.
 */

import "@testing-library/jest-dom/vitest";

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UseQueryResult } from "@tanstack/react-query";
import { MemoryRouter, useLocation } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { __maps, __resetMaps } from "../../../__mocks__/maplibre-gl";
import { useDiscoverMap } from "../../api/hooks";
import type { DiscoverMap as DiscoverMapPayload, MapCluster } from "../../api/types";
import { DiscoverMap } from "../DiscoverMap";

vi.mock("../../api/hooks", () => ({ useDiscoverMap: vi.fn() }));

/* -- fixtures ------------------------------------------------------------ */

function marker(overrides: Partial<MapCluster> = {}): MapCluster {
  return {
    coordinate: { latitude: 43.06, longitude: 141.35 },
    count: 1,
    photo_id: "01a00000-0000-7000-8000-000000000001",
    voyage_id: "01b00000-0000-7000-8000-000000000001",
    thumbnail_url: "/api/photos/01a00000-0000-7000-8000-000000000001/thumb",
    ...overrides,
  };
}

function answer(payload: Partial<DiscoverMapPayload> = {}): void {
  vi.mocked(useDiscoverMap).mockReturnValue({
    data: { clusters: [], truncated: false, ...payload },
    isPending: false,
    isError: false,
    isSuccess: true,
  } as unknown as UseQueryResult<DiscoverMapPayload>);
}

/** Reports the current address, so navigation can be asserted on rather than
 *  inferred from a spy. */
function LocationProbe() {
  const location = useLocation();
  return <span data-testid="location">{location.pathname}</span>;
}

function renderMap() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <DiscoverMap label="A map of every published photograph" />
      <LocationProbe />
    </MemoryRouter>,
  );
}

function currentPath(): string {
  return screen.getByTestId("location").textContent ?? "";
}

function mapRegion(): HTMLElement {
  return screen.getByRole("region", { name: /^A map of/ });
}

function pins(): HTMLElement[] {
  return Array.from(mapRegion().querySelectorAll<HTMLElement>("button.pv-pin"));
}

function clusters(): HTMLElement[] {
  return Array.from(mapRegion().querySelectorAll<HTMLElement>("button.pv-cluster"));
}

afterEach(() => {
  vi.clearAllMocks();
  __resetMaps();
});

/* -- what it asks for ---------------------------------------------------- */

describe("DiscoverMap — the question it asks", () => {
  /** The window has to come from the map, not from a guess. Asking about a
   *  rectangle nobody is looking at wastes a request and, on the tile server
   *  decision D4a points at, fetches beyond what the user can see. */
  it("asks about the rectangle the camera is actually showing", () => {
    answer();
    renderMap();

    const asked = vi.mocked(useDiscoverMap).mock.calls.at(-1)?.[0];
    expect(asked).not.toBeNull();

    const bounds = __maps[0].getBounds();
    expect(asked).toMatchObject({
      west: bounds.getWest(),
      south: bounds.getSouth(),
      east: bounds.getEast(),
      north: bounds.getNorth(),
    });
  });

  /**
   * Rounded, and that matters for cost rather than correctness. A pinch passes
   * through every fractional zoom between two levels; sending each one asks the
   * server a different question for an answer that changes only when the
   * grouping does.
   */
  it("rounds the zoom before making it part of the question", () => {
    answer();
    renderMap();

    const asked = vi.mocked(useDiscoverMap).mock.calls.at(-1)?.[0];
    expect(asked?.zoom).toBe(Math.round(asked?.zoom ?? -1));
  });
});

/* -- what it draws ------------------------------------------------------- */

describe("DiscoverMap — what it draws", () => {
  it("draws a pin for a marker standing for one photograph", () => {
    answer({ clusters: [marker({ count: 1 })] });
    renderMap();

    expect(pins()).toHaveLength(1);
    expect(clusters()).toHaveLength(0);
  });

  it("draws a counted bubble for a marker standing for several", () => {
    answer({ clusters: [marker({ count: 24 })] });
    renderMap();

    expect(clusters()).toHaveLength(1);
    expect(clusters()[0]).toHaveTextContent("24");
    expect(clusters()[0]).toHaveAccessibleName("24 photos here");
    expect(pins()).toHaveLength(0);
  });

  it("draws nothing at all where nothing has been published", () => {
    answer({ clusters: [] });
    renderMap();

    expect(pins()).toHaveLength(0);
    expect(clusters()).toHaveLength(0);
  });

  /**
   * A map that silently dropped half a continent looks exactly like a map of a
   * continent with fewer photographs on it, and a reader has no way to tell
   * which they are looking at.
   */
  it("says so when there were more photographs than it could show", () => {
    answer({ clusters: [marker()], truncated: true });
    renderMap();

    expect(screen.getByRole("status")).toHaveTextContent(/more photographs here/i);
  });

  it("says nothing about truncation when nothing was left out", () => {
    answer({ clusters: [marker()], truncated: false });
    renderMap();

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  /** Required by the ODbL licence, on every map instance, including this one. */
  it("shows the OpenStreetMap attribution", () => {
    answer();
    renderMap();

    expect(mapRegion().querySelector(".pv-attribution")).toHaveTextContent("OpenStreetMap");
  });
});

/* -- what a marker does -------------------------------------------------- */

describe("DiscoverMap — following a marker", () => {
  it("goes to the voyage when a single photograph is chosen", async () => {
    answer({ clusters: [marker({ count: 1, voyage_id: "v-42" })] });
    renderMap();

    expect(currentPath()).toBe("/");

    await userEvent.click(within(mapRegion()).getByRole("button", { name: /photograph/i }));

    // Asserted on the address rather than on a spy for `useNavigate`: what
    // matters is where the reader ends up, not which function was called.
    expect(currentPath()).toBe("/voyages/v-42");
  });

  /**
   * A bubble zooms in rather than navigating. The photographs behind it may
   * belong to different people's voyages, so picking one of them would be a
   * link that goes somewhere unrelated to what was clicked.
   */
  it("zooms in when a bubble is chosen instead of following one of its photographs", async () => {
    answer({ clusters: [marker({ count: 9 })] });
    renderMap();

    const before = __maps[0].getZoom();
    await userEvent.click(clusters()[0]);

    expect(__maps[0].camera.at(-1)?.kind).toBe("easeTo");
    expect(__maps[0].getZoom()).toBeGreaterThan(before);
  });
});
