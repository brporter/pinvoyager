/**
 * Behavior of the voyage detail screen.
 *
 * The API hooks are mocked rather than a server being stood up: what these
 * tests are about is what the screen does with a voyage once it has one, and
 * every state — pending, failed, empty — is a value the hook can simply return.
 *
 * There are no snapshots here on purpose. A snapshot of this screen would pass
 * whether or not selecting a pin selected its photograph, which is the only
 * thing the screen exists to do.
 */

import "@testing-library/jest-dom/vitest";

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UseQueryResult } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The substitute itself, imported by path so its bookkeeping can be reset
// between tests. `vi.mock` above is what makes the application receive it.
import { __resetMaps } from "../../../__mocks__/maplibre-gl";
import { ApiRequestError } from "../../api/client";
import { useVoyage } from "../../api/hooks";
import type { Voyage, VoyagePhoto } from "../../api/types";
import { AuthContext } from "../../auth/context";
import type { AuthState } from "../../auth/context";
import { VoyageDetail } from "../VoyageDetail";

vi.mock("../../api/hooks", () => ({ useVoyage: vi.fn() }));

// MapLibre needs a WebGL context and jsdom has none, so the real library cannot
// be constructed here at all. The substitute in `web/__mocks__/maplibre-gl.ts`
// keeps markers as real DOM elements, which is the only part of it this
// screen's behavior depends on.
vi.mock("maplibre-gl");

/* -- fixtures ------------------------------------------------------------ */

function photo(overrides: Partial<VoyagePhoto> & { id: string }): VoyagePhoto {
  return {
    position: 1,
    captured_at: "2025-02-14T09:12:00Z",
    width: 4032,
    height: 3024,
    variant_status: "ready",
    thumbnail_url: `/api/photos/${overrides.id}/thumb`,
    display_url: `/api/photos/${overrides.id}/display`,
    full_url: `/api/photos/${overrides.id}/full`,
    coordinate: { latitude: 43.06, longitude: 141.35, precision_m: 1000 },
    placed: true,
    ...overrides,
  };
}

const PLACED_A = photo({ id: "p1", position: 1 });
const PLACED_B = photo({
  id: "p2",
  position: 2,
  captured_at: "2025-02-15T14:40:00Z",
  coordinate: { latitude: 43.21, longitude: 141.5, precision_m: 1000 },
});
const UNPLACED = photo({ id: "p3", position: 3, captured_at: null, coordinate: null, placed: false });

function voyage(overrides: Partial<Voyage> = {}): Voyage {
  return {
    id: "v1",
    owner_id: "u1",
    title: "Three weeks in Hokkaido",
    description: "Late winter, mostly on foot.",
    published: true,
    granularity: "km1",
    granularity_label: "~1 km",
    precision_m: 1000,
    created_at: "2026-02-01T10:00:00Z",
    published_at: "2026-02-02T10:00:00Z",
    photo_count: 3,
    placed_count: 2,
    place_count: 2,
    unplaced_count: 1,
    photos: [PLACED_A, PLACED_B, UNPLACED],
    ...overrides,
  };
}

/* -- harness ------------------------------------------------------------- */

function queryResult(partial: Partial<UseQueryResult<Voyage>>): UseQueryResult<Voyage> {
  return {
    isPending: false,
    isError: false,
    isSuccess: true,
    refetch: vi.fn(),
    ...partial,
  } as unknown as UseQueryResult<Voyage>;
}

const ANONYMOUS: AuthState = {
  status: "anonymous",
  user: null,
  identities: [],
  error: null,
  refresh: () => Promise.resolve(),
  signOut: () => Promise.resolve(),
};

/** `useIsDesktop` reads `matchMedia`, which jsdom does not implement. Absent,
 *  the hook answers "mobile", so the desktop layout is only reachable with
 *  this in place. */
function stubViewport(desktop: boolean): void {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("min-width: 768px") ? desktop : false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }));
}

function renderScreen() {
  return render(
    <AuthContext.Provider value={ANONYMOUS}>
      <MemoryRouter initialEntries={["/voyages/v1"]}>
        <Routes>
          <Route path="/voyages/:id" element={<VoyageDetail />} />
        </Routes>
      </MemoryRouter>
    </AuthContext.Provider>,
  );
}

/**
 * Waits for the map to arrive.
 *
 * The map is code-split — `src/map/LazyVoyageMap.tsx` — so it is not in the
 * first render, and until its chunk resolves the screen shows a placeholder of
 * the same dimensions. The zoom controls are the readiness signal because they
 * exist only in the real map component, whereas the labeled region exists in
 * the placeholder too and would match immediately.
 */
async function mapReady(): Promise<void> {
  await screen.findByRole("button", { name: "Zoom in" });
}

function mapRegion(): HTMLElement {
  return screen.getByRole("region", { name: /^Map of / });
}

/**
 * The pins on the map.
 *
 * Selected by the design system's own class rather than by role, because the
 * map region also contains the two zoom controls, which are buttons and are not
 * pins. `.pv-pin` is the design's name for this thing, so a change of markup
 * that stopped applying it would be a change worth failing over.
 */
function mapPins(): HTMLElement[] {
  return Array.from(mapRegion().querySelectorAll<HTMLElement>("button.pv-pin"));
}

beforeEach(() => {
  // jsdom has no layout, so no scrolling either. The screen guards the call;
  // these tests need to observe it.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  __resetMaps();
});

/* -- tests --------------------------------------------------------------- */

describe("VoyageDetail — selection", () => {
  it("marks a photograph selected when it is chosen", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    const [first] = within(strip).getAllByRole("button");

    expect(first).toHaveAttribute("aria-pressed", "false");

    await userEvent.click(first);

    expect(first).toHaveAttribute("aria-pressed", "true");
    expect(first).toHaveAttribute("data-selected", "true");
  });

  it("highlights the pin belonging to the chosen photograph", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    const photos = within(strip).getAllByRole("button");
    const pinsBefore = mapPins();
    expect(pinsBefore).toHaveLength(2);
    expect(pinsBefore[1]).toHaveAttribute("aria-pressed", "false");

    // A photograph and its pin share an accessible name, because they are the
    // same photograph seen from two sides.
    await userEvent.click(photos[1]);

    const pin = mapRegion().querySelector<HTMLElement>(
      `button.pv-pin[aria-label="${photos[1].getAttribute("aria-label") ?? ""}"]`,
    );
    expect(pin).toHaveAttribute("aria-pressed", "true");
  });

  it("selects the photograph belonging to a chosen pin and scrolls it into view", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();

    const pin = mapPins()[1];
    const name = pin.getAttribute("aria-label") ?? "";

    await userEvent.click(pin);

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    expect(within(strip).getByRole("button", { name })).toHaveAttribute("aria-pressed", "true");
    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
  });

  it("does not scroll the list when the selection came from the list itself", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    await userEvent.click(within(strip).getAllByRole("button")[0]);

    expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  });
});

describe("VoyageDetail — precision", () => {
  it("states the precision in the header, so the pins are not read as exact", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    // Mobile: the floating header carries the badge.
    const header = screen.getByRole("heading", { level: 1 }).closest("div")?.parentElement;
    expect(header).not.toBeNull();
    expect(within(header as HTMLElement).getByText("~1 km")).toBeInTheDocument();
  });

  it("states the precision in the desktop rail header", async () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    const rail = screen.getByRole("complementary", { name: "Photos in this voyage" });
    expect(within(rail).getByText("~1 km")).toBeInTheDocument();
    expect(within(rail).getByRole("heading", { level: 1 })).toHaveTextContent(
      "Three weeks in Hokkaido",
    );
  });
});

describe("VoyageDetail — unplaced photographs", () => {
  it("accounts for photographs with no location rather than dropping them", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    // The count is stated whether or not the photographs are expanded: a user
    // who sees two pins for three photos has to be told where the third went.
    expect(screen.getByText("1 photo has no location")).toBeInTheDocument();
    expect(mapPins()).toHaveLength(2);

    await userEvent.click(screen.getByRole("button", { name: "View" }));

    const group = screen.getByRole("group", { name: "Photos with no location" });
    const [unplaced] = within(group).getAllByRole("button");
    expect(unplaced).toHaveAttribute("data-unplaced", "true");
    expect(unplaced).toHaveAccessibleName(/no location/);
  });

  it("says nothing when every photograph is placed", async () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({ photos: [PLACED_A, PLACED_B], photo_count: 2, unplaced_count: 0 }),
      }),
    );
    renderScreen();
    await mapReady();

    expect(screen.queryByText(/no location/)).not.toBeInTheDocument();
  });
});

describe("VoyageDetail — states", () => {
  it("reports that it is loading rather than rendering an empty screen", () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ isPending: true, isSuccess: false, data: undefined }),
    );
    renderScreen();

    expect(screen.getByText("Loading this voyage…")).toBeInTheDocument();
  });

  it("offers a retry and a request reference when the read fails", async () => {
    const refetch = vi.fn();
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        isError: true,
        isSuccess: false,
        error: new ApiRequestError({
          status: 500,
          code: "internal",
          message: "Something went wrong.",
          requestId: "req-42",
        }),
        refetch,
      }),
    );
    renderScreen();

    expect(screen.getByText(/req-42/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(refetch).toHaveBeenCalled();
  });

  it("says a missing voyage is missing rather than broken", () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        isError: true,
        isSuccess: false,
        error: new ApiRequestError({
          status: 404,
          code: "not_found",
          message: "No such voyage.",
          requestId: null,
        }),
      }),
    );
    renderScreen();

    expect(screen.getByText("This voyage is not here")).toBeInTheDocument();
  });

  it("has something to say about a voyage with no photographs", async () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({ photos: [], photo_count: 0, placed_count: 0, unplaced_count: 0 }),
      }),
    );
    renderScreen();
    await mapReady();

    expect(screen.getByText("No photographs yet")).toBeInTheDocument();
  });
});

describe("VoyageDetail — the mobile sheet", () => {
  it("is reachable and collapsible from the keyboard", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();

    const grip = screen.getByRole("button", { name: "Expand photo list" });
    expect(grip).toHaveAttribute("aria-expanded", "false");

    grip.focus();
    await userEvent.keyboard("{Enter}");

    expect(screen.getByRole("button", { name: "Collapse photo list" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("opens the sheet when a pin is chosen, because the point is to see the photograph", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();
    await userEvent.click(mapPins()[0]);

    expect(screen.getByRole("button", { name: "Collapse photo list" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close selection" })).toBeInTheDocument();
  });
});
