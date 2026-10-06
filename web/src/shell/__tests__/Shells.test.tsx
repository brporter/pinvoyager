import "@testing-library/jest-dom/vitest";

import type { ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ isDesktop: vi.fn(), useAuth: vi.fn() }));
vi.mock("../useIsDesktop", () => ({ useIsDesktop: mocks.isDesktop }));
vi.mock("../../auth/useAuth", () => ({ useAuth: mocks.useAuth }));

import { AppShell } from "../AppShell";
import { AuthShell } from "../AuthShell";

function route(shell: ReactNode) {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <Routes>
        <Route element={shell}>
          <Route path="/" element={<p>Current page</p>} />
        </Route>
      </Routes>
    </MemoryRouter>,
  );
}

afterEach(() => vi.clearAllMocks());

describe("responsive shells", () => {
  it("shows the mobile tab bar and hides it on desktop", () => {
    mocks.isDesktop.mockReturnValue(false);
    mocks.useAuth.mockReturnValue({ status: "anonymous", user: null });
    const { unmount } = route(<AppShell />);
    expect(document.querySelector(".pv-tabbar")).toBeInTheDocument();
    expect(screen.getByText("Current page")).toBeInTheDocument();

    unmount();
    mocks.isDesktop.mockReturnValue(true);
    route(<AppShell />);
    expect(document.querySelector(".pv-tabbar")).not.toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveAttribute("id", "pv-main");
  });

  it("shows the brand bar only in the desktop authentication shell", () => {
    mocks.isDesktop.mockReturnValue(true);
    const { unmount } = route(<AuthShell />);
    expect(screen.getByRole("navigation", { name: "Main" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "PinVoyager" })).toHaveAttribute("href", "/");

    unmount();
    mocks.isDesktop.mockReturnValue(false);
    route(<AuthShell />);
    expect(screen.queryByRole("navigation", { name: "Main" })).not.toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
  });
});
