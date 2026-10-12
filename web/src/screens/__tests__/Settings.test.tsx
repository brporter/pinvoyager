import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Settings } from "../Settings";
import { AuthContext } from "../../auth/context";
import { ADA, authState, renderScreen } from "./support";

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
  vi.unstubAllGlobals();
});

describe("Settings screen", () => {
  it("shows loading, signed-out, and recoverable read-failure states", async () => {
    const loading = renderScreen(<Settings />, { auth: authState(null, "loading") });
    expect(screen.getByRole("status")).toHaveTextContent("Loading your settings");
    loading.unmount();

    const signedOut = renderScreen(<Settings />, { auth: authState(null) });
    expect(screen.getByText("You are signed out")).toBeInTheDocument();
    signedOut.unmount();

    const refresh = vi.fn();
    renderScreen(<Settings />, {
      auth: { ...authState(null, "failed"), refresh, error: new Error("offline") },
    });
    expect(screen.getByText("Something went wrong.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("saves a changed display name", async () => {
    const user = userEvent.setup();
    const refresh = vi.fn();
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    const originalAuth = { ...authState(ADA), refresh };
    const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
    const settingsTree = (auth: typeof originalAuth) => (
      <QueryClientProvider client={client}>
        <AuthContext.Provider value={auth}>
          <MemoryRouter>
            <Settings />
          </MemoryRouter>
        </AuthContext.Provider>
      </QueryClientProvider>
    );
    const rendered = render(settingsTree(originalAuth));
    expect(screen.getByRole("link", { name: "View your profile" })).toHaveAttribute(
      "href",
      `/u/${ADA.display_name}`,
    );

    const save = screen.getByRole("button", { name: "Save name" });
    expect(save).toBeDisabled();
    const name = screen.getByRole("textbox", { name: "Display name" });
    await user.clear(name);
    await user.type(name, "ada-byron");
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    expect(fetch).toHaveBeenCalledWith(
      "/api/me",
      expect.objectContaining({
        method: "PATCH",
        credentials: "same-origin",
        body: JSON.stringify({ display_name: "ada-byron" }),
      }),
    );
    expect(refresh).toHaveBeenCalledOnce();

    rendered.rerender(
      settingsTree({ ...originalAuth, user: { ...ADA, display_name: "ada-byron" } }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("Saved.");

    rendered.rerender(settingsTree({ ...originalAuth, user: { ...ADA, display_name: null } }));
    expect(screen.getByRole("textbox", { name: "Display name" })).toHaveValue("");
  });

  it("reports a network failure while saving and lets the user retry", async () => {
    const user = userEvent.setup();
    const refresh = vi.fn();
    const fetch = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("offline"))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    renderScreen(<Settings />, { auth: { ...authState(ADA), refresh } });

    await user.clear(screen.getByRole("textbox", { name: "Display name" }));
    await user.type(screen.getByRole("textbox", { name: "Display name" }), "Ada B.");
    await user.click(screen.getByRole("button", { name: "Save name" }));

    expect(await screen.findByText(/request could not be sent/i)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save name" }));
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(2));
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("shows a client error message without relabeling it as a server failure", async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ code: "conflict", message: "That name is unavailable." }), {
          status: 409,
        }),
      ),
    );
    renderScreen(<Settings />, { auth: authState(ADA) });

    await user.clear(screen.getByRole("textbox", { name: "Display name" }));
    await user.type(screen.getByRole("textbox", { name: "Display name" }), "Ada B.");
    await user.click(screen.getByRole("button", { name: "Save name" }));

    expect(await screen.findByText("That name is unavailable.")).toBeInTheDocument();
  });

  it("shows a generic server recovery message when the request has no reference", async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ code: "internal", message: "Internal error." }), {
          status: 500,
        }),
      ),
    );
    renderScreen(<Settings />, { auth: authState(ADA) });

    const name = screen.getByRole("textbox", { name: "Display name" });
    await user.clear(name);
    await user.type(name, "Ada B.");
    await user.click(screen.getByRole("button", { name: "Save name" }));

    expect(await screen.findByText("The server failed. Please try again.")).toBeInTheDocument();
  });

  it("does not send an unchanged empty display name", () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    renderScreen(<Settings />, { auth: authState({ ...ADA, display_name: null }) });
    const name = screen.getByRole("textbox", { name: "Display name" });

    expect(name).toHaveValue("");
    fireEvent.submit(name.closest("form")!);
    expect(fetch).not.toHaveBeenCalled();
  });

  it.each([
    ["empty response body", null],
    ["malformed response body", "<html>proxy error</html>"],
    ["JSON null response body", "null"],
    ["non-string API error fields", JSON.stringify({ code: 409, message: false })],
  ])("uses the HTTP status text for a %s", async (_description, body) => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      vi.fn((_path: string, options: RequestInit) =>
        Promise.resolve(
          new Response(body, {
            status: 409,
            statusText: options.method === "DELETE" ? "Deletion refused" : "Name unavailable",
          }),
        ),
      ),
    );
    renderScreen(<Settings />, { auth: authState(ADA) });

    const name = screen.getByRole("textbox", { name: "Display name" });
    await user.clear(name);
    await user.type(name, "Ada B.");
    await user.click(screen.getByRole("button", { name: "Save name" }));

    expect(await screen.findByText("Name unavailable")).toBeInTheDocument();
  });
});
it("offers only account operations implemented by the API", () => {
  renderScreen(<Settings />, { auth: authState(ADA) });
  expect(screen.queryByRole("button", { name: /delete.*account/i })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Unlink" })).not.toBeInTheDocument();
  expect(
    screen.queryByRole("group", { name: "Default location precision" }),
  ).not.toBeInTheDocument();
});

it("shows a server failure even when it has no request reference", async () => {
  const { ApiRequestError } = await import("../../api/client");
  renderScreen(<Settings />, {
    auth: {
      ...authState(null, "failed"),
      error: new ApiRequestError({
        status: 503,
        code: "unavailable",
        message: "unavailable",
        requestId: null,
      }),
    },
  });
  expect(screen.getByText("The server failed. Please try again.")).toBeInTheDocument();
});

it("includes the request reference in a server failure", async () => {
  const { ApiRequestError } = await import("../../api/client");
  renderScreen(<Settings />, {
    auth: {
      ...authState(null, "failed"),
      error: new ApiRequestError({
        status: 503,
        code: "unavailable",
        message: "unavailable",
        requestId: "support-123",
      }),
    },
  });
  expect(
    screen.getByText("The server failed. Please try again, quoting reference support-123."),
  ).toBeInTheDocument();
});

it("offers only Google, Apple, and Microsoft account linking", () => {
  renderScreen(<Settings />, { auth: authState(ADA) });
  for (const provider of ["Google", "Apple", "Microsoft"]) {
    const row = screen.getByText(provider).closest("li")!;
    expect(within(row).getByRole("button", { name: "Link" })).toBeInTheDocument();
  }
  expect(screen.getAllByRole("button", { name: "Link" })).toHaveLength(3);
  expect(screen.queryByText("Facebook")).not.toBeInTheDocument();
});
