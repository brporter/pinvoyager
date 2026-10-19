import horizonPin from "../../../../design/logos/horizon-pin.svg";
import "@testing-library/jest-dom/vitest";

import type { ReactNode } from "react";
import { render, screen, within } from "@testing-library/react";
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

  it("shows the brand bar in the authentication shell at both widths", () => {
    mocks.isDesktop.mockReturnValue(true);
    const { unmount } = route(<AuthShell />);
    expect(screen.getByRole("navigation", { name: "Main" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "PinVoyager" })).toHaveAttribute("href", "/");

    unmount();
    mocks.isDesktop.mockReturnValue(false);
    route(<AuthShell />);
    expect(screen.getByRole("navigation", { name: "Main" })).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
  });
});

it.each([true, false])("provides legal footer links in both shells (desktop=%s)", (desktop) => {
  mocks.isDesktop.mockReturnValue(desktop);
  mocks.useAuth.mockReturnValue({ status: "anonymous", user: null });
  for (const Shell of [AppShell, AuthShell]) {
    const { unmount } = route(<Shell />);
    const footer = within(screen.getByRole("contentinfo"));
    expect(footer.getByRole("link", { name: "Terms of Service" })).toHaveAttribute(
      "href",
      "/terms",
    );
    expect(footer.getByRole("link", { name: "Privacy Policy" })).toHaveAttribute(
      "href",
      "/privacy",
    );
    unmount();
  }
});

it.each([true, false])("uses Horizon Pin in both shells (desktop=%s)", (desktop) => {
  mocks.isDesktop.mockReturnValue(desktop);
  mocks.useAuth.mockReturnValue({ status: "anonymous", user: null });
  for (const Shell of [AppShell, AuthShell]) {
    const { unmount } = route(<Shell />);
    const brand = screen.getByRole("link", { name: "PinVoyager" });
    expect(brand).toHaveAttribute("href", "/");
    const logo = brand.querySelector("img");
    expect(logo).toHaveAttribute("src", horizonPin);
    expect(logo).toHaveAttribute("alt", "");
    unmount();
  }
});
