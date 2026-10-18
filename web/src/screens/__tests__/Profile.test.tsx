/**
 * Profile — who sees a draft, and who must not know one exists.
 *
 * The draft test is the one that matters. A stranger seeing a Draft badge, a
 * grayed-out card, or a count that does not add up would undo the server's
 * choice to answer 404 rather than 403 for an unpublished voyage.
 */

import "@testing-library/jest-dom/vitest";

import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useVoyages, usePublicProfile } from "../../api/hooks";
import { Profile } from "../Profile";
import {
  ADA,
  STRANGER_ID,
  apiFailure,
  authState,
  draft,
  loadedFeed,
  feedResult,
  renderScreen,
  voyage,
} from "./support";

vi.mock("../../api/hooks", () => ({ useVoyages: vi.fn(), usePublicProfile: vi.fn() }));

const mockedUseVoyages = vi.mocked(useVoyages);

function renderProfile(
  userId: string,
  auth: import("../../auth/context").AuthState = authState(ADA),
) {
  return renderScreen(<Profile />, { path: `/u/${userId}`, route: "/u/:identifier", auth });
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Profile", () => {
  it("shows the owner their drafts, badged", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([draft(), voyage()]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByText("Draft")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /sanriku coast, unfinished/i })).toBeInTheDocument();

    // And asks for the list that contains drafts in the first place.
    expect(mockedUseVoyages).toHaveBeenCalledWith({ mine: true });
  });

  it("shows processed voyage covers alongside draft badges and leaves empty voyages without images", () => {
    mockedUseVoyages.mockReturnValue(
      loadedFeed([
        draft({ cover_url: "/api/photos/draft-cover/thumb" }),
        voyage({ cover_url: "/api/photos/public-cover/thumb" }),
        voyage({ id: "empty", title: "Empty voyage", cover_url: null }),
      ]),
    );
    const { container } = renderProfile(ADA.id, authState(ADA));
    expect(
      Array.from(container.querySelectorAll(".pv-voyage-card img"), (image) =>
        image.getAttribute("src"),
      ),
    ).toEqual(["/api/photos/draft-cover/thumb", "/api/photos/public-cover/thumb"]);
    expect(screen.getByText("Draft")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /empty voyage/i })).toBeInTheDocument();
  });

  it("has no follower count and no follow button", () => {
    // There is no social graph behind either, and the affordance without the
    // feature would be a promise the backend does not keep.
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderProfile(ADA.id);

    expect(screen.queryByRole("button", { name: /follow/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /follow/i })).not.toBeInTheDocument();
    expect(screen.queryByText(/followers?/i)).not.toBeInTheDocument();
  });

  it("names the owner and counts their drafts separately", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([draft(), voyage()]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByRole("heading", { name: "ada-lovelace", level: 1 })).toBeInTheDocument();
    expect(screen.getByText("1 voyage · 1 draft")).toBeInTheDocument();
  });

  it("uses the plural draft label when the owner has several drafts", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([draft(), draft({ id: "second-draft" })]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByText("0 voyages · 2 drafts")).toBeInTheDocument();
  });

  it("claims no total while more pages are outstanding", () => {
    // A count of what happens to have been fetched is wrong for exactly as
    // long as it is interesting.
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()], "cursor-2"));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.queryByText(/\d+ voyages?/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Load more voyages" })).toBeInTheDocument();
  });

  it("loads the next page when requested", async () => {
    const feed = loadedFeed([voyage()], "cursor-2");
    mockedUseVoyages.mockReturnValue(feed);
    renderProfile(ADA.id, authState(ADA));

    await userEvent.click(screen.getByRole("button", { name: "Load more voyages" }));

    expect(feed.fetchNextPage).toHaveBeenCalledOnce();
  });

  it("shows the owner's join month on desktop once the list is complete", () => {
    const mediaQuery = {
      matches: true,
      media: "(min-width: 768px)",
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList;
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => mediaQuery),
    );
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByText(/joined .*2026/)).toBeInTheDocument();
  });

  it("offers the owner a way to start rather than an empty grid", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByRole("heading", { name: "No voyages yet" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Upload photos" })).toHaveAttribute("href", "/upload");
  });

  it("renders the error state rather than throwing", () => {
    mockedUseVoyages.mockReturnValue(apiFailure(503, "9c1d-44a0"));

    expect(() => renderProfile(ADA.id)).not.toThrow();

    expect(
      screen.getByRole("heading", { name: "These voyages could not be loaded" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/reference 9c1d-44a0/i)).toBeInTheDocument();
  });

  it("retries from the error state", async () => {
    const failed = apiFailure();
    mockedUseVoyages.mockReturnValue(failed);
    renderProfile(ADA.id);

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(failed.refetch).toHaveBeenCalledOnce();
  });

  it("waits for the session before deciding whose profile this is", () => {
    // Rendering the public view first and swapping in the owner's would make a
    // creator's own drafts appear a beat late, which reads as data loss.
    mockedUseVoyages.mockReturnValue(loadedFeed([draft()]));
    renderProfile(ADA.id, authState(null, "loading"));

    expect(screen.getByRole("list", { name: "Loading voyages" })).toHaveAttribute(
      "aria-busy",
      "true",
    );
    expect(screen.queryByText("Draft")).not.toBeInTheDocument();
  });

  it("labels an owner's avatar when the account has no name or picture", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([]));
    renderProfile(ADA.id, authState({ ...ADA, display_name: null, avatar_url: null }));

    expect(screen.getByRole("img", { name: "Your account" })).toBeInTheDocument();
  });
});

it("loads the requested stranger's public profile without scanning discovery", () => {
  vi.mocked(usePublicProfile).mockReturnValue({
    ...feedResult(),
    data: {
      pages: [{ user: { ...ADA, id: STRANGER_ID }, voyages: [voyage()], next_before: null }],
      pageParams: [null],
    },
  } as unknown as ReturnType<typeof usePublicProfile>);
  renderProfile(STRANGER_ID, authState(ADA));
  expect(mockedUseVoyages).not.toHaveBeenCalled();
  expect(usePublicProfile).toHaveBeenCalledWith(STRANGER_ID);
  expect(screen.getByRole("link", { name: /Three weeks in Hokkaido/i })).toBeInTheDocument();
});

it("shows loading while the owner's voyage query is pending", () => {
  mockedUseVoyages.mockReturnValue({ isPending: true } as ReturnType<typeof useVoyages>);
  renderProfile(ADA.id);
  expect(screen.getByRole("list", { name: "Loading voyages" })).toHaveAttribute(
    "aria-busy",
    "true",
  );
});
