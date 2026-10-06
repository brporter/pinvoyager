import "@testing-library/jest-dom/vitest";

import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

const auth = vi.hoisted(() => ({ useAuth: vi.fn() }));
vi.mock("../../auth/useAuth", () => ({ useAuth: auth.useAuth }));

import type { AuthState } from "../../auth/context";
import type { CurrentUser } from "../../api/types";
import { TopNav } from "../TopNav";

const user: CurrentUser = {
  id: "user-1",
  display_name: "Ada Lovelace",
  avatar_url: null,
  created_at: "2026-01-01T00:00:00Z",
};

function state(status: AuthState["status"], currentUser: CurrentUser | null): AuthState {
  return {
    status,
    user: currentUser,
    identities: [],
    error: null,
    refresh: vi.fn(async () => {}),
    signOut: vi.fn(async () => {}),
  };
}

function renderNav(desktop: boolean, status: AuthState["status"], currentUser: CurrentUser | null) {
  auth.useAuth.mockReturnValue(state(status, currentUser));
  return render(
    <MemoryRouter initialEntries={["/voyages?sort=recent"]}>
      <TopNav desktop={desktop} />
    </MemoryRouter>,
  );
}

afterEach(() => vi.clearAllMocks());

describe("connected TopNav", () => {
  it("shows public navigation and sign-in to an anonymous desktop visitor", () => {
    renderNav(true, "anonymous", null);

    expect(screen.getByRole("link", { name: "Discover" })).toHaveAttribute("href", "/");
    expect(screen.getByRole("link", { name: "Sign in" })).toHaveAttribute(
      "href",
      "/signin?redirect_to=%2Fvoyages%3Fsort%3Drecent",
    );
    expect(screen.queryByRole("link", { name: /New voyage/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /account/i })).not.toBeInTheDocument();
  });

  it("shows the voyage action and account menu to an authenticated desktop visitor", () => {
    renderNav(true, "authenticated", user);

    expect(screen.getByRole("link", { name: /My voyages/ })).toHaveAttribute("href", "/u/user-1");
    expect(screen.getByRole("link", { name: /New voyage/ })).toHaveAttribute("href", "/upload");
    expect(screen.getByRole("button", { name: /Ada Lovelace/ })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Sign in" })).not.toBeInTheDocument();
  });

  it("omits desktop links and the new-voyage action on mobile", () => {
    renderNav(false, "authenticated", user);

    expect(screen.queryByRole("link", { name: /My voyages/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /New voyage/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Ada Lovelace/ })).toBeInTheDocument();
  });

  it("offers sign-in after an auth check fails but suppresses it while loading", () => {
    const { unmount } = renderNav(true, "failed", null);
    expect(screen.getByRole("link", { name: "Sign in" })).toBeInTheDocument();

    unmount();
    renderNav(true, "loading", null);
    expect(screen.queryByRole("link", { name: "Sign in" })).not.toBeInTheDocument();
  });
});
