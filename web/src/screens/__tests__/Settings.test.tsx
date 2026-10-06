/**
 * Tests for the stored default location precision.
 *
 * This value decides how precisely a *new* voyage publishes its coordinates,
 * for a user who never opens the precision control at all. That makes the
 * fallback a privacy decision rather than a formatting one: if an unreadable or
 * absent stored value resolved to `exact`, a user who had never touched the
 * setting would be publishing their exact positions because of a corrupted
 * localStorage entry.
 */

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { readDefaultGranularity } from "../Settings";
import { Settings } from "../Settings";
import { AuthContext } from "../../auth/context";
import { ADA, authState, renderScreen } from "./support";

const KEY = "pinvoyager.default_granularity";

/** The setting a user gets when nothing usable is stored. Deliberately not
 *  `exact`. */
const CAUTIOUS_DEFAULT = "km1";

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
  vi.unstubAllGlobals();
});

describe("readDefaultGranularity", () => {
  it("returns a stored value that is one of the four settings", () => {
    window.localStorage.setItem(KEY, "km50");
    expect(readDefaultGranularity()).toBe("km50");
  });

  it("returns exact when the user has genuinely chosen it", () => {
    window.localStorage.setItem(KEY, "exact");
    expect(readDefaultGranularity()).toBe("exact");
  });

  it("falls back to a coarse setting when nothing is stored", () => {
    expect(readDefaultGranularity()).toBe(CAUTIOUS_DEFAULT);
  });

  /** The important one. A value that is not a recognized setting — an older
   *  spelling, a truncated write, something another script put there — must
   *  not be treated as permission to publish exact coordinates. */
  it("falls back to a coarse setting rather than exact when the stored value is unrecognized", () => {
    for (const junk of ["", "EXACT", "km5", "true", "null", "1km", "{}"]) {
      window.localStorage.setItem(KEY, junk);
      const resolved = readDefaultGranularity();

      expect(
        resolved,
        `stored value ${JSON.stringify(junk)} must not resolve to exact`,
      ).toBe(CAUTIOUS_DEFAULT);
    }
  });

  /** Safari in a private window throws on localStorage access rather than
   *  returning null. Reading the setting must not take the settings page down
   *  with it, and must still fail toward the cautious answer. */
  it("falls back to a coarse setting when storage throws instead of returning null", () => {
    vi.spyOn(window, "localStorage", "get").mockImplementation(() => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    });

    expect(readDefaultGranularity()).toBe(CAUTIOUS_DEFAULT);
  });
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

  it("saves a changed display name and persists the selected precision locally", async () => {
    const user = userEvent.setup();
    const refresh = vi.fn();
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    window.localStorage.setItem(KEY, "km50");
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

    const save = screen.getByRole("button", { name: "Save name" });
    expect(save).toBeDisabled();
    const name = screen.getByRole("textbox", { name: "Display name" });
    await user.clear(name);
    await user.type(name, "Ada Byron");
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    expect(fetch).toHaveBeenCalledWith("/api/me", expect.objectContaining({
      method: "PATCH",
      credentials: "same-origin",
      body: JSON.stringify({ display_name: "Ada Byron" }),
    }));
    expect(refresh).toHaveBeenCalledOnce();

    rendered.rerender(
      settingsTree({ ...originalAuth, user: { ...ADA, display_name: "Ada Byron" } }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("Saved.");

    rendered.rerender(
      settingsTree({ ...originalAuth, user: { ...ADA, display_name: null } }),
    );
    expect(screen.getByRole("textbox", { name: "Display name" })).toHaveValue("");

    const precision = screen.getByRole("group", { name: "Default location precision" });
    expect(within(precision).getByRole("button", { name: "~50 km" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await user.click(within(precision).getByRole("button", { name: "Exact" }));
    expect(window.localStorage.getItem(KEY)).toBe("exact");
    expect(within(precision).getByRole("button", { name: "Exact" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("reports a network failure while saving and lets the user retry", async () => {
    const user = userEvent.setup();
    const refresh = vi.fn();
    const fetch = vi.fn()
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
        new Response(
          JSON.stringify({ code: "conflict", message: "That name is unavailable." }),
          { status: 409 },
        ),
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
      vi.fn().mockResolvedValue(
        new Response(body, { status: 409, statusText: "Name unavailable" }),
      ),
    );
    renderScreen(<Settings />, { auth: authState(ADA) });

    const name = screen.getByRole("textbox", { name: "Display name" });
    await user.clear(name);
    await user.type(name, "Ada B.");
    await user.click(screen.getByRole("button", { name: "Save name" }));

    expect(await screen.findByText("Name unavailable")).toBeInTheDocument();
  });

  it("explains why the only identity cannot be unlinked and supports canceling deletion", async () => {
    const user = userEvent.setup();
    renderScreen(<Settings />, {
      auth: {
        ...authState(ADA),
        identities: [{
          provider: "google",
          linked_at: "2026-01-01T00:00:00Z",
          last_seen_at: "2026-01-01T00:00:00Z",
        }],
      },
    });

    const providers = screen.getByRole("heading", { name: "Sign-in methods" }).closest("section")!;
    expect(within(providers).getByRole("button", { name: "Unlink" })).toBeDisabled();
    expect(within(providers).getByText("This is your only way in")).toBeInTheDocument();
    expect(within(providers).getAllByRole("button", { name: "Link" })).toHaveLength(3);

    await user.click(screen.getByRole("button", { name: "Delete my account" }));
    expect(screen.getByText("Delete this account permanently?")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Keep my account" }));
    expect(screen.queryByText("Delete this account permanently?")).not.toBeInTheDocument();
  });

  it("allows unlinking one provider when another recovery method remains", async () => {
    const user = userEvent.setup();
    const refresh = vi.fn();
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    renderScreen(<Settings />, {
      auth: {
        ...authState(ADA),
        refresh,
        identities: [
          { provider: "google", linked_at: "2026-01-01T00:00:00Z", last_seen_at: "invalid" },
          {
            provider: "apple",
            linked_at: "2026-01-01T00:00:00Z",
            last_seen_at: new Date(Date.now() - 86_400_000).toISOString(),
          },
        ],
      },
    });

    expect(screen.getByText("Linked · last used recently")).toBeInTheDocument();
    expect(screen.getByText("Linked · last used yesterday")).toBeInTheDocument();
    const providers = screen.getByRole("heading", { name: "Sign-in methods" }).closest("section")!;
    const unlinkButtons = within(providers).getAllByRole("button", { name: "Unlink" });
    expect(unlinkButtons).toHaveLength(2);
    expect(unlinkButtons[0]).toBeEnabled();
    await user.click(unlinkButtons[0]);

    await waitFor(() => expect(fetch).toHaveBeenCalledWith(
      "/auth/link/google",
      expect.objectContaining({ method: "DELETE", credentials: "same-origin" }),
    ));
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("formats each elapsed-day range for linked providers", () => {
    vi.setSystemTime(new Date("2026-09-27T12:00:00Z"));
    const ago = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString();
    renderScreen(<Settings />, {
      auth: {
        ...authState(ADA),
        identities: [
          { provider: "google", linked_at: ago(0), last_seen_at: ago(0) },
          { provider: "apple", linked_at: ago(0), last_seen_at: ago(2) },
          { provider: "microsoft", linked_at: ago(0), last_seen_at: ago(7) },
          { provider: "facebook", linked_at: ago(0), last_seen_at: ago(65) },
        ],
      },
    });

    expect(screen.getByText("Linked · last used today")).toBeInTheDocument();
    expect(screen.getByText("Linked · last used 2 days ago")).toBeInTheDocument();
    expect(screen.getByText("Linked · last used 1 week ago")).toBeInTheDocument();
    expect(screen.getByText("Linked · last used 2 months ago")).toBeInTheDocument();

    const monthUser = authState(ADA);
    monthUser.identities = [
      { provider: "google", linked_at: ago(0), last_seen_at: ago(30) },
      { provider: "apple", linked_at: ago(0), last_seen_at: ago(30) },
    ];
    const monthView = renderScreen(<Settings />, { auth: monthUser });
    expect(screen.getAllByText("Linked · last used 1 month ago")).toHaveLength(2);
    monthView.unmount();

    const weekUser = authState(ADA);
    weekUser.identities = [
      { provider: "google", linked_at: ago(0), last_seen_at: ago(14) },
      { provider: "apple", linked_at: ago(0), last_seen_at: ago(14) },
    ];
    const weekView = renderScreen(<Settings />, { auth: weekUser });
    expect(screen.getAllByText("Linked · last used 2 weeks ago")).toHaveLength(2);
    weekView.unmount();
  });

  it("shows an unlink request error and marks the active provider busy", async () => {
    const user = userEvent.setup();
    let finishRequest!: (response: Response) => void;
    const fetch = vi.fn(() => new Promise<Response>((resolve) => { finishRequest = resolve; }));
    vi.stubGlobal("fetch", fetch);
    renderScreen(<Settings />, {
      auth: {
        ...authState(ADA),
        identities: [
          { provider: "google", linked_at: "2026-01-01T00:00:00Z", last_seen_at: "2026-01-01T00:00:00Z" },
          { provider: "apple", linked_at: "2026-01-01T00:00:00Z", last_seen_at: "2026-01-01T00:00:00Z" },
        ],
      },
    });

    const unlink = within(screen.getByRole("heading", { name: "Sign-in methods" }).closest("section")!)
      .getAllByRole("button", { name: "Unlink" })[0];
    await user.click(unlink);
    await waitFor(() => {
      expect(screen.getAllByRole("button", { name: "Unlink" })[0])
        .toHaveAttribute("aria-busy", "true");
    });
    finishRequest(new Response(JSON.stringify({ code: "internal", message: "Internal error." }), {
      status: 500,
    }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That provider could not be unlinked");
    expect(fetch).toHaveBeenCalledWith("/auth/link/google", expect.any(Object));
  });

  it("returns to the signed-out app after account deletion succeeds", async () => {
    const user = userEvent.setup();
    const assign = vi.fn();
    vi.stubGlobal("location", { assign });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 204 })));
    renderScreen(<Settings />, { auth: authState(ADA) });

    await user.click(screen.getByRole("button", { name: "Delete my account" }));
    await user.click(screen.getByRole("button", { name: "Yes, delete my account permanently" }));

    await waitFor(() => expect(assign).toHaveBeenCalledWith("/"));
  });

  it("keeps an account when its deletion request fails and shows the request reference", async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ code: "internal", message: "Internal error." }),
          { status: 500, headers: { "x-request-id": "delete-ref-7" } },
        ),
      ),
    );
    renderScreen(<Settings />, { auth: authState(ADA) });

    await user.click(screen.getByRole("button", { name: "Delete my account" }));
    await user.click(screen.getByRole("button", { name: "Yes, delete my account permanently" }));

    expect(await screen.findByText(/quoting reference delete-ref-7/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Keep my account" })).toBeEnabled();
  });
});
