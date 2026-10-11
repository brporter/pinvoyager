import { screen } from "@testing-library/react";
import { useLocation } from "react-router";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { usePublicProfile } from "../../api/hooks";
import { PublicProfile } from "../PublicProfile";
import { ADA, apiFailure, feedResult, renderScreen, voyage } from "./support";
vi.mock("../../api/hooks", async (original) => ({
  ...(await original<typeof import("../../api/hooks")>()),
  usePublicProfile: vi.fn(),
}));
const mock = vi.mocked(usePublicProfile);
const ready = (overrides = {}) =>
  ({
    ...feedResult(),
    data: {
      pages: [
        { user: { ...ADA, display_name: "betaporter" }, voyages: [voyage()], next_before: null },
      ],
      pageParams: [null],
    },
    ...overrides,
  }) as unknown as ReturnType<typeof usePublicProfile>;
function CurrentPath() {
  return <span data-testid="path">{useLocation().pathname}</span>;
}
function show() {
  return renderScreen(
    <>
      <PublicProfile />
      <CurrentPath />
    </>,
    { path: "/u/betaporter", route: "/u/:identifier" },
  );
}
beforeEach(() => {
  vi.clearAllMocks();
});
describe("Public display-name profile", () => {
  it("shows a named public profile anonymously and loads additional pages", async () => {
    const feed = ready({ hasNextPage: true });
    mock.mockReturnValue(feed);
    show();
    expect(screen.getByRole("heading", { name: "betaporter" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Three weeks in Hokkaido/i })).toBeInTheDocument();
    expect(screen.queryByText("Draft")).not.toBeInTheDocument();
    expect(mock).toHaveBeenCalledWith("betaporter");
    await userEvent.click(screen.getByRole("button", { name: "Load more voyages" }));
    expect(feed.fetchNextPage).toHaveBeenCalledOnce();
  });
  it("shows loading, missing-profile, and retryable error states", async () => {
    mock.mockReturnValue({ ...feedResult({ isPending: true }) } as unknown as ReturnType<
      typeof usePublicProfile
    >);
    const loading = show();
    expect(screen.getByRole("status")).toHaveTextContent("Loading profile");
    loading.unmount();
    mock.mockReturnValue(apiFailure(404) as unknown as ReturnType<typeof usePublicProfile>);
    const missing = show();
    expect(screen.getByText("Profile not found")).toBeInTheDocument();
    missing.unmount();
    const failed = apiFailure();
    mock.mockReturnValue(failed as unknown as ReturnType<typeof usePublicProfile>);
    show();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(failed.refetch).toHaveBeenCalledOnce();
  });
  it("shows a real empty public profile without exposing private voyages", () => {
    mock.mockReturnValue(
      ready({
        data: {
          pages: [{ user: { ...ADA, display_name: "betaporter" }, voyages: [], next_before: null }],
          pageParams: [null],
        },
      }),
    );
    show();
    expect(screen.getByText("No public voyages")).toBeInTheDocument();
  });
  it("handles network errors and incomplete profile labels", async () => {
    const feed = feedResult({ isError: true, error: new Error("offline") });
    mock.mockReturnValue(feed as unknown as ReturnType<typeof usePublicProfile>);
    const failed = show();
    expect(screen.getByText("This profile could not be loaded")).toBeInTheDocument();
    failed.unmount();
    mock.mockReturnValue(
      ready({
        hasNextPage: true,
        data: {
          pages: [{ user: { ...ADA, display_name: null }, voyages: [], next_before: "cursor" }],
          pageParams: [null],
        },
      }),
    );
    show();
    expect(screen.getByRole("heading", { name: "Voyages" })).toBeInTheDocument();
    expect(screen.queryByText("No public voyages")).not.toBeInTheDocument();
  });
  it("replaces a historical URL with its canonical profile URL", () => {
    mock.mockReturnValue(
      ready({
        data: {
          pages: [{ user: { ...ADA, display_name: "new-name" }, voyages: [], next_before: null }],
          pageParams: [null],
        },
      }),
    );
    show();
    expect(screen.getByTestId("path")).toHaveTextContent("/u/new-name");
  });
});
