/**
 * Discover — behavior, not appearance.
 *
 * The four things worth pinning down are the ones that are easy to get wrong
 * and invisible when they are: the precision label on every card, the empty
 * state instead of a bare grid, an error rendered rather than thrown, and the
 * cursor pagination actually asking for the next page.
 */

import "@testing-library/jest-dom/vitest";

import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useVoyages } from "../../api/hooks";
import { Discover, monthYear, referenceOf, VoyageGridSkeleton } from "../Discover";
import { useIsDesktop } from "../../shell/useIsDesktop";
import {
  ADA,
  apiFailure,
  authState,
  feedResult,
  loadedFeed,
  renderScreen,
  voyage,
} from "./support";

vi.mock("../../api/hooks", () => ({ useVoyages: vi.fn() }));
// This suite tests the feed screen's states and actions; map behavior has its
// own coverage in `web/src/map/__tests__/DiscoverMap.test.tsx`.
vi.mock("../../map", () => ({
  LazyDiscoverMap: () => null,
  LazyVoyageMap: () => null,
}));
vi.mock("../../shell/useIsDesktop", () => ({ useIsDesktop: vi.fn(() => false) }));

const mockedUseVoyages = vi.mocked(useVoyages);
const mockedUseIsDesktop = vi.mocked(useIsDesktop);

beforeEach(() => {
  vi.clearAllMocks();
  mockedUseIsDesktop.mockReturnValue(false);
});

describe("Discover", () => {
  it("omits a month label for an invalid publication date", () => {
    expect(monthYear("not-a-date")).toBe("");
  });

  it("uses the requested skeleton count when one is provided", () => {
    const { container } = renderScreen(<VoyageGridSkeleton desktop count={4} />);
    expect(container.querySelectorAll(".pv-voyage-card")).toHaveLength(4);
  });

  it("sizes the default skeleton grid for desktop and mobile", () => {
    const desktop = renderScreen(<VoyageGridSkeleton desktop />);
    expect(desktop.container.querySelectorAll(".pv-voyage-card")).toHaveLength(6);
    desktop.unmount();

    const mobile = renderScreen(<VoyageGridSkeleton desktop={false} />);
    expect(mobile.container.querySelectorAll(".pv-voyage-card")).toHaveLength(2);
  });

  it("returns an API request reference and ignores other errors", () => {
    expect(referenceOf(apiFailure(500, "req-123").error)).toBe("req-123");
    expect(referenceOf(new Error("offline"))).toBeNull();
  });

  it("states the mechanism in the hero rather than a slogan", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderScreen(<Discover />);

    // What the product does and where the data comes from — the thing a
    // visitor needs to understand before uploading anything.
    expect(
      screen.getByRole("heading", { name: /photographs, placed where they were taken/i, level: 1 }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/reads the location your camera recorded/i),
    ).toBeInTheDocument();
  });

  it("uses the desktop hero and three-column layout when the viewport is wide", () => {
    mockedUseIsDesktop.mockReturnValue(true);
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderScreen(<Discover />);

    expect(screen.getByRole("link", { name: "Browse voyages" })).toHaveAttribute(
      "href",
      "#pv-discover-recent",
    );
    expect(screen.getByRole("heading", { name: "Everywhere people have been" })
      .parentElement?.getAttribute("style"))
      .toContain("var(--pv-space-6)");
  });

  it("shows each voyage's precision label on its card", () => {
    // The design's annotation 1: a browser scanning the grid should see how
    // precise a creator publishes before the map loads.
    mockedUseVoyages.mockReturnValue(
      loadedFeed([
        voyage({ id: "a", title: "Coastal, in fog", granularity_label: "~10 km" }),
        voyage({
          id: "b",
          title: "Islands, slowly",
          granularity: "exact",
          granularity_label: "Exact",
          precision_m: null,
        }),
      ]),
    );
    renderScreen(<Discover />);

    const fog = screen.getByRole("link", { name: /coastal, in fog/i });
    expect(within(fog).getByText("~10 km")).toBeInTheDocument();

    const islands = screen.getByRole("link", { name: /islands, slowly/i });
    expect(within(islands).getByText("Exact")).toBeInTheDocument();
  });

  it("links each card to that voyage", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage({ id: "abc" })]));
    renderScreen(<Discover />);

    expect(screen.getByRole("link", { name: /three weeks in hokkaido/i })).toHaveAttribute(
      "href",
      "/voyages/abc",
    );
  });

  it("shows the designed empty state when there is nothing to discover", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([]));
    renderScreen(<Discover />);

    expect(screen.getByRole("heading", { name: "No public voyages yet" })).toBeInTheDocument();
    expect(screen.getByText(/every voyage on pinvoyager is public/i)).toBeInTheDocument();
    // Every state offers a way onward.
    expect(screen.getAllByRole("link", { name: "Create a voyage" }).length).toBeGreaterThan(0);
  });

  it("renders the error state rather than throwing, and quotes the reference", () => {
    mockedUseVoyages.mockReturnValue(apiFailure(500, "7f3a-2b91"));

    expect(() => renderScreen(<Discover />)).not.toThrow();

    expect(
      screen.getByRole("heading", { name: "Voyages could not be loaded" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/reference 7f3a-2b91/i)).toBeInTheDocument();
    // The real cause stays in the server log.
    expect(screen.queryByText(/Something went wrong\./)).not.toBeInTheDocument();
  });

  it("retries from the error state", async () => {
    const failed = apiFailure();
    mockedUseVoyages.mockReturnValue(failed);
    renderScreen(<Discover />);

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(failed.refetch).toHaveBeenCalledOnce();
  });

  it("draws card-shaped skeletons while the first page is in flight", () => {
    mockedUseVoyages.mockReturnValue(feedResult({ isPending: true }));
    const { container } = renderScreen(<Discover />);

    // The region says "loading" once; the skeletons themselves are hidden.
    expect(screen.getByRole("list", { name: "Loading voyages" })).toHaveAttribute(
      "aria-busy",
      "true",
    );
    // Cover frame plus two text lines per card, so nothing moves on arrival.
    const cards = container.querySelectorAll(".pv-voyage-card");
    expect(cards.length).toBe(2);
    for (const card of cards) {
      expect(card).toHaveAttribute("aria-hidden", "true");
      // The 4:3 cover and the two text lines the title and meta will occupy.
      expect(card.querySelectorAll(".pv-skeleton").length).toBe(3);
    }
  });

  it("asks for the next page by cursor, and offers nothing to click at the end", async () => {
    const more = loadedFeed([voyage()], "cursor-2");
    mockedUseVoyages.mockReturnValue(more);
    const { unmount } = renderScreen(<Discover />);

    await userEvent.click(screen.getByRole("button", { name: "Load more voyages" }));
    expect(more.fetchNextPage).toHaveBeenCalledOnce();

    unmount();
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderScreen(<Discover />);

    expect(screen.queryByRole("button", { name: "Load more voyages" })).not.toBeInTheDocument();
  });

  it("sends a signed-out visitor to sign in before uploading, carrying the destination", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderScreen(<Discover />);

    expect(screen.getByRole("link", { name: "Create a voyage" })).toHaveAttribute(
      "href",
      "/signin?redirect_to=%2Fupload",
    );
  });

  it("sends a signed-in visitor straight to upload", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderScreen(<Discover />, { auth: authState(ADA) });

    expect(screen.getByRole("link", { name: "Create a voyage" })).toHaveAttribute(
      "href",
      "/upload",
    );
  });
});
