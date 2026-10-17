/**
 * Checks detail-screen states, map/list selection, and gallery behavior.
 * API hooks supply fixtures, and the MapLibre double provides DOM markers
 * and camera events without drawing tiles.
 */

import "@testing-library/jest-dom/vitest";

import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UseQueryResult } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The substitute itself, imported by path so its bookkeeping can be reset
// between tests. `vi.mock` above is what makes the application receive it.
import { __resetMaps } from "../../../__mocks__/maplibre-gl";
import { ApiRequestError } from "../../api/client";
import { useUpdateVoyage, useVoyage } from "../../api/hooks";
import type { Voyage, VoyagePhoto } from "../../api/types";
import { AuthContext } from "../../auth/context";
import type { AuthState } from "../../auth/context";
import { VoyageDetail } from "../VoyageDetail";

vi.mock("../../api/hooks", () => ({ useVoyage: vi.fn(), useUpdateVoyage: vi.fn() }));

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
const UNPLACED = photo({
  id: "p3",
  position: 3,
  captured_at: null,
  coordinate: null,
  placed: false,
});

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

function screenTree(auth: AuthState = ANONYMOUS) {
  return (
    <AuthContext.Provider value={auth}>
      <MemoryRouter initialEntries={["/voyages/v1"]}>
        <Routes>
          <Route path="/voyages/:id" element={<VoyageDetail />} />
        </Routes>
      </MemoryRouter>
    </AuthContext.Provider>
  );
}

function renderScreen(auth: AuthState = ANONYMOUS) {
  return render(screenTree(auth));
}

function renderWithoutVoyageId() {
  return render(
    <AuthContext.Provider value={ANONYMOUS}>
      <MemoryRouter initialEntries={["/not-a-voyage"]}>
        <Routes>
          <Route path="*" element={<VoyageDetail />} />
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
  vi.mocked(useUpdateVoyage).mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
    error: null,
  } as unknown as ReturnType<typeof useUpdateVoyage>);
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

  it("shows the selected photo state when it has no coordinate", async () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({
          photos: [photo({ ...PLACED_A, coordinate: null })],
          photo_count: 1,
          placed_count: 1,
          place_count: 1,
          unplaced_count: 0,
        }),
      }),
    );
    renderScreen();
    await mapReady();

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    await userEvent.click(within(strip).getByRole("button"));
    expect(screen.getByText(/No location/)).toBeInTheDocument();
  });

  it("does not scroll the list when the selection came from the list itself", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    await userEvent.click(within(strip).getAllByRole("button")[0]);

    expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  });

  it("clears a photo selection when that photo is chosen again", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    const strip = screen.getByRole("group", { name: "Photos in this voyage" });
    const photoButton = within(strip).getAllByRole("button")[0];
    await userEvent.click(photoButton);
    expect(photoButton).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(photoButton);
    expect(photoButton).toHaveAttribute("aria-pressed", "false");
  });

  it("clears a pin selection with the close control", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();
    await userEvent.click(mapPins()[0]);

    await userEvent.click(screen.getByRole("button", { name: "Close selection" }));
    expect(screen.getByRole("button", { name: "Collapse photo list" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Close selection" })).not.toBeInTheDocument();
  });
});

describe("VoyageDetail — precision", () => {
  it.each([false, true])(
    "omits precision from photo captions and accessible names at desktop=%s",
    async (desktop) => {
      stubViewport(desktop);
      vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
      const view = renderScreen();
      await mapReady();
      const labelsAbsent = () => {
        expect(view.container.textContent).not.toMatch(/Exact|~1 km|~10 km|~50 km/);
        for (const node of view.container.querySelectorAll("[aria-label], [alt]")) {
          expect(`${node.getAttribute("aria-label")} ${node.getAttribute("alt")}`).not.toMatch(
            /Exact|~1 km|~10 km|~50 km/,
          );
        }
      };
      labelsAbsent();
      fireEvent.click(view.container.querySelector<HTMLElement>('.pv-photo[role="button"]')!);
      labelsAbsent();
    },
  );

  it("omits precision from the mobile voyage header", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    // Mobile floating header keeps the voyage title without a precision badge.
    const header = screen.getByRole("heading", { level: 1 }).closest("div")?.parentElement;
    expect(header).not.toBeNull();
    expect(within(header as HTMLElement).queryByText("~1 km")).not.toBeInTheDocument();
  });

  it("omits precision from the desktop rail header", async () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    const rail = screen.getByRole("complementary", { name: "Photos in this voyage" });
    expect(within(rail).queryByText("~1 km")).not.toBeInTheDocument();
    expect(within(rail).getByRole("heading", { level: 1 })).toHaveTextContent(
      "Three weeks in Hokkaido",
    );
  });
});

describe("VoyageDetail — date groups", () => {
  it("groups photos from the same capture date under one heading", async () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({
          photos: [
            PLACED_A,
            photo({
              ...PLACED_B,
              id: "same-day",
              captured_at: "2025-02-14T18:30:00Z",
            }),
          ],
        }),
      }),
    );
    renderScreen();
    await mapReady();

    expect(screen.getAllByRole("heading", { name: "February 14" })).toHaveLength(1);
  });

  it("keeps photos with missing or invalid capture dates in an unknown-date group", async () => {
    const missingDate = photo({ id: "no-date", position: 1, captured_at: null });
    const invalidDate = photo({ id: "bad-date", position: 2, captured_at: "not-a-date" });
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({ photos: [missingDate, invalidDate], photo_count: 2, placed_count: 2 }),
      }),
    );
    stubViewport(true);
    const desktop = renderScreen();
    await mapReady();
    expect(screen.getByRole("heading", { name: "Date unknown" })).toBeInTheDocument();
    desktop.unmount();

    stubViewport(false);
    renderScreen();
    await mapReady();
    await userEvent.click(screen.getAllByRole("button", { name: /Photograph/ })[0]);
    expect(screen.getByText("Capture time unknown")).toBeInTheDocument();
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

  it("uses the plural count when several photographs have no location", async () => {
    const unplaced = [
      UNPLACED,
      photo({
        id: "p4",
        position: 4,
        coordinate: null,
        placed: false,
        captured_at: null,
      }),
    ];
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({
          photos: [PLACED_A, ...unplaced],
          photo_count: 3,
          placed_count: 1,
          unplaced_count: 2,
        }),
      }),
    );
    renderScreen();
    await mapReady();

    expect(screen.getByText("2 photos have no location")).toBeInTheDocument();
  });
});

describe("VoyageDetail — states", () => {
  it("shows the missing state when the route has no voyage id", () => {
    renderWithoutVoyageId();
    expect(screen.getByText("This voyage is not here")).toBeInTheDocument();
  });

  it("reports that it is loading rather than rendering an empty screen", () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ isPending: true, isSuccess: false, data: undefined }),
    );
    renderScreen();

    expect(screen.getByText("Loading this voyage…")).toBeInTheDocument();
  });

  it("shows the desktop skeleton rail while loading", () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ isPending: true, isSuccess: false, data: undefined }),
    );
    renderScreen();
    expect(document.querySelectorAll(".pv-photo--square")).toHaveLength(4);
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

  it("shows a generic read error when the failure is not an API error", () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ isError: true, isSuccess: false, error: new Error("offline") }),
    );
    renderScreen();

    expect(screen.getByText("This voyage could not be loaded")).toBeInTheDocument();
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

  it("labels a draft in the mobile sheet", async () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({ published: false }),
      }),
    );
    renderScreen();
    await mapReady();

    expect(screen.getByText("Draft")).toBeInTheDocument();
  });

  it("shows an empty photo rail on desktop and labels an unpublished voyage as a draft", async () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        data: voyage({
          photos: [],
          published: false,
          photo_count: 0,
          placed_count: 0,
          unplaced_count: 0,
        }),
      }),
    );
    renderScreen();
    await mapReady();

    expect(screen.getAllByText("No photographs yet").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Draft").length).toBeGreaterThan(0);
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
    await userEvent.keyboard(" ");
    expect(screen.getByRole("button", { name: "Expand photo list" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: "Expand photo list" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("opens the sheet when a pin is chosen, because the point is to see the photograph", async () => {
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query === "(prefers-reduced-motion: reduce)",
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
    }));
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();

    await mapReady();
    await userEvent.click(mapPins()[0]);

    expect(screen.getByRole("button", { name: "Collapse photo list" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close selection" })).toBeInTheDocument();
    expect(Element.prototype.scrollIntoView).toHaveBeenCalledWith({
      block: "nearest",
      inline: "nearest",
      behavior: "auto",
    });
  });

  it("supports arrow keys to expand and collapse the sheet", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    const grip = screen.getByRole("button", { name: "Expand photo list" });
    grip.focus();
    await userEvent.keyboard("{ArrowUp}");
    expect(screen.getByRole("button", { name: "Collapse photo list" })).toBeInTheDocument();
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.getByRole("button", { name: "Expand photo list" })).toBeInTheDocument();
  });

  it("resizes the sheet while dragging and snaps to the nearest rest position", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    const grip = screen.getByRole("button", { name: "Expand photo list" });
    fireEvent.pointerDown(grip, { pointerId: 1, clientY: 300 });
    fireEvent.pointerMove(grip, { pointerId: 1, clientY: 100 });
    fireEvent.pointerUp(grip, { pointerId: 1, clientY: 100 });
    expect(screen.getByRole("button", { name: "Collapse photo list" })).toBeInTheDocument();

    fireEvent.pointerDown(grip, { pointerId: 1, clientY: 100 });
    fireEvent.pointerMove(grip, { pointerId: 1, clientY: 300 });
    fireEvent.pointerUp(grip, { pointerId: 1, clientY: 300 });
    expect(screen.getByRole("button", { name: "Expand photo list" })).toBeInTheDocument();
  });

  it("ignores pointer moves and releases when no drag is active", async () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    await mapReady();

    const grip = screen.getByRole("button", { name: "Expand photo list" });
    fireEvent.pointerMove(grip, { pointerId: 2, clientY: 20 });
    fireEvent.pointerUp(grip, { pointerId: 2, clientY: 20 });
    expect(grip).toHaveAttribute("aria-expanded", "false");
  });
});

describe("VoyageDetail — ownership", () => {
  it("keeps the publish action visible and explains a failed publish", () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage({ published: false }) }));
    vi.mocked(useUpdateVoyage).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      error: new Error("A voyage name is required"),
    } as unknown as ReturnType<typeof useUpdateVoyage>);
    renderScreen({
      ...ANONYMOUS,
      status: "authenticated",
      user: {
        id: "u1",
        display_name: "Owner",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00Z",
      },
    });
    expect(screen.getByText("A voyage name is required")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Publish voyage" })).toBeInTheDocument();
  });

  it.each([true, false])(
    "offers a prominent publish action for an owner's draft (desktop=%s)",
    async (desktop) => {
      stubViewport(desktop);
      const mutate = vi.fn();
      vi.mocked(useUpdateVoyage).mockReturnValue({
        mutate,
        isPending: false,
        error: null,
      } as unknown as ReturnType<typeof useUpdateVoyage>);
      vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage({ published: false }) }));
      renderScreen({
        ...ANONYMOUS,
        status: "authenticated",
        user: {
          id: "u1",
          display_name: "Owner",
          avatar_url: null,
          created_at: "2026-01-01T00:00:00Z",
        },
      });
      const button = await screen.findByRole("button", { name: "Publish voyage" });
      expect(button).toHaveClass("pv-btn--primary");
      await userEvent.setup().click(button);
      expect(mutate).toHaveBeenCalledWith({ published: true });
    },
  );
  it("shows the edit action to the voyage owner", async () => {
    stubViewport(true);
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    const view = renderScreen({
      ...ANONYMOUS,
      status: "authenticated",
      user: {
        id: "u1",
        display_name: "Owner",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00Z",
      },
    });

    await mapReady();
    expect(screen.getByRole("link", { name: "Edit" })).toHaveAttribute("href", "/voyages/v1/edit");
    view.unmount();
  });
});

describe("VoyageDetail — location gallery", () => {
  it.each([false, true])(
    "opens only clustered photos in voyage order (desktop=%s)",
    async (desktop) => {
      stubViewport(desktop);
      vi.mocked(useVoyage).mockReturnValue(
        queryResult({
          data: voyage({
            photos: [PLACED_B, { ...PLACED_A, coordinate: PLACED_B.coordinate }, UNPLACED],
          }),
        }),
      );
      renderScreen();
      await mapReady();
      const marker = mapRegion().querySelector<HTMLButtonElement>(".pv-cluster")!;
      await userEvent.click(marker);
      const dialog = screen.getByRole("dialog", { name: "Photos at this location" });
      expect(within(dialog).getByRole("img")).toHaveAttribute("src", PLACED_B.display_url);
      expect(within(dialog).getByText("1 of 2")).toBeInTheDocument();
      await userEvent.click(within(dialog).getByRole("button", { name: "Next photo" }));
      expect(within(dialog).getByRole("img")).toHaveAttribute("src", PLACED_A.display_url);
      await userEvent.click(within(dialog).getByRole("button", { name: "Close gallery" }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(marker).toHaveFocus();
    },
  );
});

it.each([false, true])(
  "opens a share dialog for anonymous visitors and returns focus on close (desktop=%s)",
  async (desktop) => {
    stubViewport(desktop);
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
    renderScreen();
    const share = screen.getByRole("button", { name: "Share voyage" });
    await userEvent.click(share);
    const dialog = screen.getByRole("dialog", { name: "Share voyage" });
    expect(within(dialog).getByRole("textbox", { name: "Voyage URL" })).toHaveValue(
      `${window.location.origin}/voyages/v1`,
    );
    await userEvent.click(within(dialog).getByRole("button", { name: "Close share dialog" }));
    expect(screen.queryByRole("dialog", { name: "Share voyage" })).not.toBeInTheDocument();
    expect(share).toHaveFocus();
  },
);

it.each(["u1", "another-user"])(
  "offers draft publication only to its owner when sharing as %s",
  async (userId) => {
    stubViewport(true);
    const owner = userId === "u1";
    const mutate = vi.fn((_patch, options) => options?.onSuccess());
    vi.mocked(useUpdateVoyage).mockReturnValue({
      mutate,
      isPending: false,
      error: null,
    } as unknown as ReturnType<typeof useUpdateVoyage>);
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage({ published: !owner }) }));
    renderScreen({
      ...ANONYMOUS,
      status: "authenticated",
      user: {
        id: userId,
        display_name: "Visitor",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00Z",
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Share voyage" }));
    const dialog = screen.getByRole("dialog", { name: "Share voyage" });
    if (owner) {
      expect(within(dialog).queryByRole("textbox", { name: "Voyage URL" })).not.toBeInTheDocument();
      await userEvent.click(within(dialog).getByRole("button", { name: "Publish" }));
      expect(mutate).toHaveBeenCalledWith({ published: true }, expect.any(Object));
    } else {
      expect(within(dialog).queryByRole("button", { name: "Publish" })).not.toBeInTheDocument();
    }
    expect(within(dialog).getByRole("textbox", { name: "Voyage URL" })).toHaveValue(
      `${window.location.origin}/voyages/v1`,
    );
  },
);

it("restores focus to the selected pin after closing its gallery", async () => {
  vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
  renderScreen();
  await mapReady();
  const pin = mapPins()[1];
  const label = pin.getAttribute("aria-label");
  await userEvent.click(pin);
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getAllByRole("img")).toHaveLength(1);
  await userEvent.click(within(dialog).getByRole("button", { name: "Close gallery" }));
  expect(
    mapPins().find((candidate) => candidate.getAttribute("aria-label") === label),
  ).toHaveFocus();
});

it("can close the gallery if a refreshed voyage no longer contains its photo", async () => {
  vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage() }));
  const view = renderScreen();
  await mapReady();
  await userEvent.click(mapPins()[1]);
  vi.mocked(useVoyage).mockReturnValue(
    queryResult({ data: voyage({ photos: [], photo_count: 0 }) }),
  );
  view.rerender(screenTree());
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText("0 photos")).toBeInTheDocument();
  await userEvent.click(within(dialog).getByRole("button", { name: "Close gallery" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

describe("VoyageDetail — desktop photo pages", () => {
  function pagedVoyage() {
    stubViewport(true);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      return {
        width: 388,
        height: this.classList.contains("pv-voyage-page-area") ? 250 : 28,
      } as DOMRect;
    });
    const photos = Array.from({ length: 7 }, (_, index) =>
      photo({
        id: `paged-${index}`,
        position: index + 1,
        captured_at: `2025-02-14T09:${String(index).padStart(2, "0")}:00Z`,
        coordinate: { latitude: 40 + index, longitude: 140 + index, precision_m: 1000 },
      }),
    );
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage({ photos, photo_count: 7 }) }));
    return renderScreen();
  }

  afterEach(() => vi.restoreAllMocks());

  it("fits the map to the viewport and pages photos forward and backward", async () => {
    const view = pagedVoyage();
    const rail = screen.getByRole("complementary", { name: "Photos in this voyage" });
    expect(view.container.querySelector(".pv-voyage-desktop")).toBeInTheDocument();
    expect(within(rail).getAllByRole("img")).toHaveLength(2);
    expect(within(rail).queryByRole("link", { name: "Previous" })).not.toBeInTheDocument();
    await userEvent.click(within(rail).getByRole("link", { name: "More" }));
    expect(within(rail).getAllByRole("img")[0]).toHaveAttribute("src", "/api/photos/paged-2/thumb");
    await userEvent.click(within(rail).getByRole("link", { name: "Previous" }));
    expect(within(rail).getAllByRole("img")[0]).toHaveAttribute("src", "/api/photos/paged-0/thumb");
    for (let i = 0; i < 3; i++)
      await userEvent.click(within(rail).getByRole("link", { name: "More" }));
    expect(within(rail).getAllByRole("img")).toHaveLength(1);
    expect(within(rail).queryByRole("link", { name: "More" })).not.toBeInTheDocument();
  });

  it("recalculates pages when the rail resizes and preserves the current photo", async () => {
    let resize!: ResizeObserverCallback;
    const disconnect = vi.fn();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: ResizeObserverCallback) {
          resize = callback;
        }
        observe = vi.fn();
        disconnect = disconnect;
      },
    );
    const view = pagedVoyage();
    const rail = screen.getByRole("complementary", { name: "Photos in this voyage" });
    await userEvent.click(within(rail).getByRole("link", { name: "More" }));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      return {
        width: 388,
        height: this.classList.contains("pv-voyage-page-area") ? 650 : 38,
      } as DOMRect;
    });
    act(() => resize([], {} as ResizeObserver));
    expect(within(rail).getAllByRole("img")).toHaveLength(6);
    expect(within(rail).getByText("Page 1 of 2")).toBeInTheDocument();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      return {
        width: 388,
        height: this.classList.contains("pv-voyage-page-area") ? 150 : 38,
      } as DOMRect;
    });
    fireEvent(window, new Event("resize"));
    expect(within(rail).getAllByRole("img")).toHaveLength(2);
    expect(within(rail).getByText("Page 2 of 4")).toBeInTheDocument();
    view.unmount();
    expect(disconnect).toHaveBeenCalled();
  });

  it("includes unplaced photos in pagination and returns to page one if photos disappear", async () => {
    const view = pagedVoyage();
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ data: voyage({ photos: [PLACED_A, UNPLACED] }) }),
    );
    view.rerender(screenTree());
    const rail = screen.getByRole("complementary", { name: "Photos in this voyage" });
    await userEvent.click(within(rail).getByRole("link", { name: "More" }));
    expect(
      within(rail).getByRole("group", { name: "Photos with no location" }),
    ).toBeInTheDocument();
    expect(within(rail).getByRole("button", { name: /no location/ })).toHaveAttribute(
      "data-unplaced",
      "true",
    );
    vi.mocked(useVoyage).mockReturnValue(queryResult({ data: voyage({ photos: [PLACED_A] }) }));
    view.rerender(screenTree());
    expect(within(rail).getByText("Page 1 of 1")).toBeInTheDocument();
    expect(within(rail).queryByRole("link")).not.toBeInTheDocument();
  });

  it("shows the page containing a selected map pin", async () => {
    pagedVoyage();
    await mapReady();
    await userEvent.click(mapPins()[6]);
    const rail = screen.getByRole("complementary", { name: "Photos in this voyage" });
    expect(within(rail).getAllByRole("img")[0]).toHaveAttribute("src", "/api/photos/paged-6/thumb");
    expect(
      within(rail).getByRole("button", { name: mapPins()[6].getAttribute("aria-label")! }),
    ).toHaveAttribute("aria-pressed", "true");
  });
});
