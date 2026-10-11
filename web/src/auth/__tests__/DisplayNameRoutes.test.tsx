import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "../../App";
import { voyage, draft, ADA, STRANGER_ID } from "../../screens/__tests__/support";
import { WithQueryClient } from "./support";
afterEach(() => vi.unstubAllGlobals());
describe("display-name routes", () => {
  it.each(["betaporter", ADA.id])(
    "serves published voyages anonymously at /u/%s",
    async (identifier) => {
      const fetch = vi.fn(async (input: RequestInfo | URL) => {
        if (String(input) === "/api/me")
          return new Response(JSON.stringify({ code: "unauthorized", message: "Sign in" }), {
            status: 401,
          });
        if (String(input) === `/api/profiles/${identifier}`)
          return new Response(
            JSON.stringify({
              user: { ...ADA, display_name: "betaporter" },
              voyages: [voyage()],
              next_before: null,
            }),
          );
        throw new Error(`Unexpected request: ${String(input)}`);
      });
      vi.stubGlobal("fetch", fetch);
      render(
        <WithQueryClient>
          <MemoryRouter initialEntries={[`/u/${identifier}`]}>
            <App />
          </MemoryRouter>
        </WithQueryClient>,
      );
      expect(await screen.findByRole("heading", { name: "betaporter" })).toBeInTheDocument();
      expect(screen.getByRole("link", { name: /Three weeks in Hokkaido/i })).toBeInTheDocument();
      expect(screen.getByRole("link", { name: "Sign in" })).toBeInTheDocument();
    },
  );
  it("serves settings independently of profile routes", async () => {
    const fetch = vi.fn(
      async () =>
        new Response(
          JSON.stringify({ user: { ...ADA, display_name: "betaporter" }, identities: [] }),
        ),
    );
    vi.stubGlobal("fetch", fetch);
    render(
      <WithQueryClient>
        <MemoryRouter initialEntries={["/settings"]}>
          <App />
        </MemoryRouter>
      </WithQueryClient>,
    );
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    expect(fetch).toHaveBeenCalledWith("/api/me", expect.anything());
  });
});

function CurrentPath() {
  return <span data-testid="path">{useLocation().pathname}</span>;
}

it.each([ADA.id, ADA.display_name!, "ADA-LOVELACE", "old-name", "renamed-account"])(
  "shows the owner drafts and creation controls at /u/%s",
  async (identifier) => {
    const fetch = vi.fn(async (input: RequestInfo | URL) => {
      const path = String(input);
      if (path === "/api/me") return new Response(JSON.stringify({ user: ADA, identities: [] }));
      if (path === "/api/profiles/renamed-account")
        return new Response(
          JSON.stringify({
            user: { ...ADA, display_name: "renamed-account" },
            voyages: [voyage()],
            next_before: null,
          }),
        );
      if (path === "/api/profiles/old-name")
        return new Response(JSON.stringify({ user: ADA, voyages: [voyage()], next_before: null }));
      if (path === "/api/voyages?mine=true")
        return new Response(JSON.stringify({ voyages: [draft(), voyage()], next_before: null }));
      throw new Error(`Unexpected request: ${path}`);
    });
    vi.stubGlobal("fetch", fetch);
    render(
      <WithQueryClient>
        <MemoryRouter initialEntries={[`/u/${identifier}`]}>
          <App />
          <CurrentPath />
        </MemoryRouter>
      </WithQueryClient>,
    );
    expect(await screen.findByText("Draft")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "New voyage" })).toBeInTheDocument();
    if (identifier === "old-name")
      expect(screen.getByTestId("path")).toHaveTextContent(`/u/${ADA.display_name}`);
  },
);

it.each([STRANGER_ID, "other-creator"])(
  "shows only published voyages for another account at /u/%s",
  async (identifier) => {
    const fetch = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input) === "/api/me")
        return new Response(JSON.stringify({ user: ADA, identities: [] }));
      if (String(input) === `/api/profiles/${identifier}`)
        return new Response(
          JSON.stringify({
            user: { ...ADA, id: STRANGER_ID, display_name: "other-creator" },
            voyages: [voyage({ owner_id: STRANGER_ID })],
            next_before: null,
          }),
        );
      throw new Error(`Unexpected request: ${String(input)}`);
    });
    vi.stubGlobal("fetch", fetch);
    render(
      <WithQueryClient>
        <MemoryRouter initialEntries={[`/u/${identifier}`]}>
          <App />
        </MemoryRouter>
      </WithQueryClient>,
    );
    expect(await screen.findByRole("heading", { name: "other-creator" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Three weeks in Hokkaido/i })).toBeInTheDocument();
    expect(screen.queryByText("Draft")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "New voyage" })).not.toBeInTheDocument();
    expect(fetch.mock.calls.some(([input]) => String(input).includes("mine=true"))).toBe(false);
  },
);

it("does not resolve a display name at the root", async () => {
  const fetch = vi.fn(
    async () =>
      new Response(JSON.stringify({ code: "unauthorized", message: "Sign in" }), { status: 401 }),
  );
  vi.stubGlobal("fetch", fetch);
  render(
    <WithQueryClient>
      <MemoryRouter initialEntries={["/betaporter"]}>
        <App />
      </MemoryRouter>
    </WithQueryClient>,
  );
  expect(await screen.findByRole("heading", { name: "Nothing here" })).toBeInTheDocument();
  await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
  expect(fetch).toHaveBeenCalledWith("/api/me", expect.anything());
});
