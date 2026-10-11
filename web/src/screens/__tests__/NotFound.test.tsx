/**
 * 404 and error states.
 *
 * The load-bearing assertion is that a deleted voyage and a draft belonging to
 * somebody else produce the same words. The server answers 404 for both so
 * that unpublished work cannot be enumerated by probing ids; a screen that
 * distinguished them would undo that in the interface instead of the API.
 */

import "@testing-library/jest-dom/vitest";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { ReactElement } from "react";

import { ErrorScreen, NotFound, VoyageUnavailable } from "../NotFound";

function renderRouted(element: ReactElement) {
  return render(<MemoryRouter>{element}</MemoryRouter>);
}

describe("NotFound", () => {
  it("says the address does not exist, and offers a way onward", () => {
    renderRouted(<NotFound />);

    expect(screen.getByRole("heading", { name: "Nothing here" })).toBeInTheDocument();
    expect(screen.getByText(/this page does not exist/i)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Go to Discover" })).toHaveAttribute("href", "/");
  });

  it("is reachable from the keyboard", async () => {
    renderRouted(<NotFound />);

    await userEvent.tab();
    expect(screen.getByRole("link", { name: "Go to Discover" })).toHaveFocus();
  });
});

describe("VoyageUnavailable", () => {
  it("stays vague, so a deleted voyage and a hidden draft read alike", () => {
    renderRouted(<VoyageUnavailable />);

    expect(
      screen.getByRole("heading", { name: "This voyage is no longer available" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("It may have been deleted or made private by the person who created it."),
    ).toBeInTheDocument();
  });

  it("offers no way to ask the owner for access", () => {
    // "Request access" would confirm that a voyage exists at this address,
    // which is precisely what the 404-not-403 rule refuses to do.
    renderRouted(<VoyageUnavailable />);

    expect(screen.queryByText(/request access/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    // Nor does it name the two cases it is covering for. "Draft" here would
    // tell a prober they had found one.
    expect(screen.queryByText(/\bdraft\b|\bunpublished\b/i)).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Browse other voyages" })).toHaveAttribute("href", "/");
  });
});

describe("ErrorScreen", () => {
  it("quotes a reference and no internal detail", () => {
    renderRouted(<ErrorScreen reference="7f3a-2b91" standalone />);

    expect(screen.getByRole("heading", { name: "Something went wrong" })).toBeInTheDocument();
    expect(screen.getByText("Reference 7f3a-2b91")).toBeInTheDocument();
    // No status code, no stack, no server message.
    expect(screen.queryByText(/500|internal|stack|exception/i)).not.toBeInTheDocument();
  });

  it("says whose fault it is, because a 500 makes people stop trying", () => {
    renderRouted(<ErrorScreen standalone />);

    expect(screen.getByText(/this is our fault, not yours/i)).toBeInTheDocument();
  });

  it("omits the reference line when the request never reached the service", () => {
    renderRouted(<ErrorScreen reference={null} standalone />);

    expect(screen.queryByText(/^reference/i)).not.toBeInTheDocument();
  });

  it("retries when it can, and points at Discover when it cannot", async () => {
    const onRetry = vi.fn();
    const { unmount } = renderRouted(<ErrorScreen onRetry={onRetry} />);

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();

    unmount();
    renderRouted(<ErrorScreen />);

    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Go to Discover" })).toBeInTheDocument();
  });
});
