/**
 * Profile — who sees a draft, and who must not know one exists.
 *
 * The draft test is the one that matters. A stranger seeing a Draft badge, a
 * greyed-out card, or a count that does not add up would undo the server's
 * choice to answer 404 rather than 403 for an unpublished voyage.
 */

import "@testing-library/jest-dom/vitest";

import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useVoyages } from "../../api/hooks";
import { Profile } from "../Profile";
import {
  ADA,
  STRANGER_ID,
  apiFailure,
  authState,
  draft,
  feedResult,
  loadedFeed,
  renderScreen,
  voyage,
} from "./support";

vi.mock("../../api/hooks", () => ({ useVoyages: vi.fn() }));

const mockedUseVoyages = vi.mocked(useVoyages);

function renderProfile(userId: string, auth = authState(null)) {
  return renderScreen(<Profile />, { path: `/u/${userId}`, route: "/u/:userId", auth });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("Profile", () => {
  it("shows the owner their drafts, badged", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([draft(), voyage()]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByText("Draft")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: /sanriku coast, unfinished/i }),
    ).toBeInTheDocument();

    // And asks for the list that contains drafts in the first place.
    expect(mockedUseVoyages).toHaveBeenCalledWith({ mine: true });
  });

  it("shows a stranger no draft badge and no draft", () => {
    // The public feed never returns one; the screen must not invent a place
    // for it either.
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()]));
    renderProfile(ADA.id);

    expect(screen.queryByText("Draft")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /three weeks in hokkaido/i })).toBeInTheDocument();
    expect(mockedUseVoyages).toHaveBeenCalledWith({});
  });

  it("does not badge a draft that leaked into a stranger's feed", () => {
    // Belt and braces: even if the API ever returned one, this view would not
    // announce which side of the publish line it is on.
    mockedUseVoyages.mockReturnValue(loadedFeed([draft(), voyage()]));
    renderProfile(ADA.id);

    expect(screen.queryByText("Draft")).not.toBeInTheDocument();
  });

  it("lists only that creator's voyages out of the public feed", () => {
    mockedUseVoyages.mockReturnValue(
      loadedFeed([
        voyage({ id: "mine", title: "Coastal, in fog" }),
        voyage({ id: "theirs", owner_id: STRANGER_ID, title: "Desert roads" }),
      ]),
    );
    renderProfile(ADA.id);

    expect(screen.getByRole("link", { name: /coastal, in fog/i })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /desert roads/i })).not.toBeInTheDocument();
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

    expect(screen.getByRole("heading", { name: "Ada Lovelace", level: 1 })).toBeInTheDocument();
    expect(screen.getByText("1 voyage · 1 draft")).toBeInTheDocument();
  });

  it("claims no total while more pages are outstanding", () => {
    // A count of what happens to have been fetched is wrong for exactly as
    // long as it is interesting.
    mockedUseVoyages.mockReturnValue(loadedFeed([voyage()], "cursor-2"));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.queryByText(/\d+ voyages?/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Load more voyages" })).toBeInTheDocument();
  });

  it("shows the empty state only once the feed is exhausted", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([]));
    renderProfile(ADA.id);

    expect(screen.getByRole("heading", { name: "No public voyages" })).toBeInTheDocument();
  });

  it("offers the owner a way to start rather than an empty grid", () => {
    mockedUseVoyages.mockReturnValue(loadedFeed([]));
    renderProfile(ADA.id, authState(ADA));

    expect(screen.getByRole("heading", { name: "No voyages yet" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Upload photos" })).toHaveAttribute("href", "/upload");
  });

  it("keeps a page of a stranger's feed from claiming the creator has nothing", () => {
    // None of this page belongs to them, but there are more pages to come.
    mockedUseVoyages.mockReturnValue(
      loadedFeed([voyage({ owner_id: STRANGER_ID })], "cursor-2"),
    );
    renderProfile(ADA.id);

    expect(screen.queryByRole("heading", { name: "No public voyages" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Load more voyages" })).toBeInTheDocument();
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

  it("copes with a creator who has no name and no picture", () => {
    // An Apple-only account whose first authorization was not captured has
    // neither, and a stranger's name is not on the wire at all.
    mockedUseVoyages.mockReturnValue(feedResult({ isPending: true }));
    renderProfile(STRANGER_ID);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "This creator" })).toBeInTheDocument();
  });
});
