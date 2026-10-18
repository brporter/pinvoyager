import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CurrentUser } from "../../api/types";
import { AccountMenu } from "../AccountMenu";

const mocks = vi.hoisted(() => ({ signOut: vi.fn() }));

vi.mock("../../auth/useAuth", () => ({
  useAuth: () => ({ signOut: mocks.signOut }),
}));

const namedUser: CurrentUser = {
  id: "user-1",
  display_name: "Sam Example",
  avatar_url: null,
  is_site_admin: false,
  created_at: "2025-01-01T00:00:00Z",
};

function LocationProbe() {
  const location = useLocation();
  return <output data-testid="location">{location.pathname}</output>;
}

function renderMenu(user: CurrentUser = namedUser) {
  return render(
    <MemoryRouter initialEntries={["/settings"]}>
      <AccountMenu user={user} />
      <LocationProbe />
    </MemoryRouter>,
  );
}

afterEach(() => {
  mocks.signOut.mockReset();
  vi.restoreAllMocks();
});

describe("AccountMenu", () => {
  it("names the account button and opens a disclosure with settings and sign out", async () => {
    const user = userEvent.setup();
    renderMenu();

    const trigger = screen.getByRole("button", { name: "Account — signed in as Sam Example" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("link", { name: "Settings" })).not.toBeInTheDocument();

    await user.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(trigger).toHaveAttribute("aria-controls");
    expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute("href", "/settings");
    expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
  });

  it("uses a generic accessible name when the user has no display name", () => {
    renderMenu({ ...namedUser, display_name: null });
    expect(screen.getByRole("button", { name: "Your account" })).toBeInTheDocument();
  });

  it("keeps the panel open for pointer events inside and dismisses it outside", async () => {
    const user = userEvent.setup();
    renderMenu();
    const trigger = screen.getByRole("button", { name: /signed in as Sam Example/ });
    await user.click(trigger);

    fireEvent.pointerDown(screen.getByRole("link", { name: "Settings" }));
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    fireEvent.pointerDown(document.body);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("dismisses a pointer event whose target is not a DOM node", async () => {
    const user = userEvent.setup();
    const addListener = vi.spyOn(document, "addEventListener");
    renderMenu();
    const trigger = screen.getByRole("button", { name: /signed in as Sam Example/ });
    await user.click(trigger);

    const listener = addListener.mock.calls.find(([type]) => type === "pointerdown")?.[1] as
      EventListener | undefined;
    expect(listener).toBeTypeOf("function");
    act(() => listener?.({ target: {} } as PointerEvent));

    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("closes on Escape and returns focus to the trigger", async () => {
    const user = userEvent.setup();
    renderMenu();
    const trigger = screen.getByRole("button", { name: /signed in as Sam Example/ });
    await user.click(trigger);
    fireEvent.keyDown(trigger, { key: "Tab" });
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    screen.getByRole("link", { name: "Settings" }).focus();

    fireEvent.keyDown(screen.getByRole("link", { name: "Settings" }), { key: "Escape" });

    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).toHaveFocus();
  });

  it("does not close when focus moves within the panel, then closes when focus leaves", async () => {
    const user = userEvent.setup();
    renderMenu();
    const trigger = screen.getByRole("button", { name: /signed in as Sam Example/ });
    await user.click(trigger);
    const settings = screen.getByRole("link", { name: "Settings" });

    fireEvent.blur(trigger, { relatedTarget: settings });
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    fireEvent.blur(trigger, { relatedTarget: document.body });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("closes after Settings navigation", async () => {
    const user = userEvent.setup();
    renderMenu();
    const trigger = screen.getByRole("button", { name: /signed in as Sam Example/ });
    await user.click(trigger);
    await user.click(screen.getByRole("link", { name: "Settings" }));

    expect(screen.getByTestId("location")).toHaveTextContent("/settings");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("signs out and replaces the current route with Discover", async () => {
    const user = userEvent.setup();
    mocks.signOut.mockResolvedValue(undefined);
    renderMenu();
    const trigger = screen.getByRole("button", { name: /signed in as Sam Example/ });
    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent("/"));
    expect(mocks.signOut).toHaveBeenCalledOnce();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });
});

it("links administrators to administration and closes the account menu", async () => {
  renderMenu({ ...namedUser, is_site_admin: true });
  await userEvent.click(screen.getByRole("button", { name: /Account/ }));
  await userEvent.click(screen.getByRole("link", { name: "Site administration" }));
  expect(screen.getByTestId("location")).toHaveTextContent("/admin");
  expect(screen.queryByRole("link", { name: "Site administration" })).not.toBeInTheDocument();
});

it("keeps the route and offers retry after logout fails", async () => {
  mocks.signOut.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(undefined);
  const user = userEvent.setup();
  renderMenu();
  await user.click(screen.getByRole("button", { name: /Account/ }));
  await user.click(screen.getByRole("button", { name: "Sign out" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not sign out");
  expect(screen.getByTestId("location")).toHaveTextContent("/settings");
  await user.click(screen.getByRole("button", { name: "Retry sign out" }));
  await waitFor(() => expect(screen.getByTestId("location").textContent).toBe("/"));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
