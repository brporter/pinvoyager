import "@testing-library/jest-dom/vitest";

import type { ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { Avatar } from "../Avatar";
import { initialsOf } from "../initials";
import { TabBar } from "../TabBar";
import { TopNav } from "../TopNav";

function at(path: string, ui: ReactNode) {
  return render(<MemoryRouter initialEntries={[path]}>{ui}</MemoryRouter>);
}

describe("TopNav", () => {
  it("offers a signed-out visitor a way in without demanding it", () => {
    at("/", <TopNav />);

    // Discover is still reachable — published voyages are public, so this is an
    // anonymous browsing route.
    expect(screen.getByRole("link", { name: "Discover" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/signin");
  });

  it("calls a sign-in handler instead of routing when one is given", async () => {
    const onSignIn = vi.fn();
    at("/", <TopNav onSignIn={onSignIn} />);

    await userEvent.click(screen.getByRole("button", { name: "Sign in" }));

    expect(onSignIn).toHaveBeenCalledOnce();
  });

  it("shows the signed-in controls and names the avatar", () => {
    at("/", <TopNav user={{ displayName: "Bryan Porter", avatarUrl: null }} />);

    expect(screen.getByRole("link", { name: /New voyage/ })).toHaveAttribute("href", "/upload");
    expect(screen.getByRole("img", { name: "Signed in as Bryan Porter" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Sign in" })).not.toBeInTheDocument();
  });

  it("copes with a user who has neither a name nor a picture", () => {
    // Apple returns the name only on first authorization, so this is an
    // ordinary account state rather than an edge case.
    at("/", <TopNav user={{ displayName: null, avatarUrl: null }} />);

    expect(screen.getByRole("img", { name: "Signed in" })).toBeInTheDocument();
  });

  it("marks the current page for the screen reader and the stylesheet at once", () => {
    at(
      "/upload",
      <TopNav
        links={[
          { label: "Discover", to: "/", end: true },
          { label: "Upload", to: "/upload" },
        ]}
      />,
    );

    expect(screen.getByRole("link", { name: "Upload" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Discover" })).not.toHaveAttribute("aria-current");
  });

  it("does not treat every route as Discover", () => {
    at("/settings", <TopNav />);

    expect(screen.getByRole("link", { name: "Discover" })).not.toHaveAttribute("aria-current");
  });

  it("exposes a navigation landmark", () => {
    at("/", <TopNav />);

    expect(screen.getByRole("navigation", { name: "Main" })).toBeInTheDocument();
  });
});

describe("TabBar", () => {
  it("gives every tab a text label as well as an icon", () => {
    at("/", <TabBar />);

    for (const name of ["Discover", "Voyages", "Upload", "Profile"]) {
      expect(screen.getByRole("link", { name })).toBeInTheDocument();
    }
  });

  it("marks the current tab", () => {
    at("/upload", <TabBar />);

    expect(screen.getByRole("link", { name: "Upload" })).toHaveAttribute("aria-current", "page");
  });

  it("reaches every tab by keyboard", async () => {
    at("/", <TabBar />);

    await userEvent.tab();
    expect(screen.getByRole("link", { name: "Discover" })).toHaveFocus();
    await userEvent.tab();
    expect(screen.getByRole("link", { name: "Voyages" })).toHaveFocus();
  });
});

describe("Avatar", () => {
  it("derives initials from a display name", () => {
    expect(initialsOf("Bryan Porter")).toBe("BP");
    expect(initialsOf("Bryan Michael Porter")).toBe("BP");
    expect(initialsOf("Prince")).toBe("P");
    expect(initialsOf("  spaced   out  ")).toBe("SO");
    expect(initialsOf("😀 Porter")).toBe("😀P");
  });

  it("returns no initials when there is no name to derive them from", () => {
    expect(initialsOf(null)).toBeNull();
    expect(initialsOf("")).toBeNull();
    expect(initialsOf("   ")).toBeNull();
  });

  it("falls back to initials when there is no picture", () => {
    render(<Avatar name="Bryan Porter" label="Bryan Porter" />);

    expect(screen.getByRole("img", { name: "Bryan Porter" })).toHaveTextContent("BP");
  });

  it("shows the picture when there is one, without a second name for it", () => {
    const { container } = render(
      <Avatar name="Bryan Porter" src="https://example.test/a.png" label="Bryan Porter" />,
    );

    const image = container.querySelector("img");
    expect(image).toHaveAttribute("src", "https://example.test/a.png");
    // The wrapper already carries the name; an alt would announce it twice.
    expect(image).toHaveAttribute("alt", "");
    expect(screen.getAllByRole("img", { name: "Bryan Porter" })).toHaveLength(1);
  });

  it("stays out of the accessibility tree when a visible name sits beside it", () => {
    render(
      <>
        <Avatar name="Bryan Porter" />
        <span>Bryan Porter</span>
      </>,
    );

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });
});
