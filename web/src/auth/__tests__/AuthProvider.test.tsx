/**
 * What a 401 from `/api/me` means.
 *
 * This is the load-bearing distinction in the whole module. Most of this
 * application is readable without an account, so "nobody is signed in" is an
 * ordinary answer, and treating it as an error is how an anonymous visitor ends
 * up looking at an error screen on the home page. The mirror-image mistake is
 * just as bad: reporting a 500 as "signed out" shows a signed-in user the
 * anonymous site and offers them a sign-in button they do not need.
 */

import userEvent from "@testing-library/user-event";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { isApiError } from "../../api/client";
import { AuthProvider } from "../AuthProvider";
import { useAuth } from "../useAuth";
import { SIGNED_IN, WithQueryClient, stubMe } from "./support";

function Probe() {
  const { status, user, error } = useAuth();
  return (
    <>
      <span data-testid="status">{status}</span>
      <span data-testid="user">{user?.display_name ?? "none"}</span>
      <span data-testid="error">{error === null ? "none" : error.message}</span>
      <span data-testid="error-status">{isApiError(error) ? String(error.status) : "none"}</span>
    </>
  );
}

function renderProbe() {
  return render(
    <WithQueryClient>
      <AuthProvider>
        <Probe />
      </AuthProvider>
    </WithQueryClient>,
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("AuthProvider", () => {
  it("treats a 401 as the anonymous state, not as an error", async () => {
    stubMe(401);
    renderProbe();

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("anonymous");
    });

    expect(screen.getByTestId("user")).toHaveTextContent("none");
    // The point of the test. Anonymous is an ordinary state; nothing failed.
    expect(screen.getByTestId("error")).toHaveTextContent("none");
  });

  it("reports the signed-in user when the session resolves", async () => {
    stubMe(200, SIGNED_IN);
    renderProbe();

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("authenticated");
    });

    expect(screen.getByTestId("user")).toHaveTextContent("ada-lovelace");
  });

  it("requires an initial display name, saves it, and resumes the original screen", async () => {
    const user = userEvent.setup();
    let chosen: string | null = null;
    const fetch = vi.fn(async (_input: RequestInfo | URL, options?: RequestInit) => {
      if (options?.method === "PATCH") {
        chosen = JSON.parse(String(options.body)).display_name;
        return new Response(null, { status: 204 });
      }
      return new Response(
        JSON.stringify({ ...SIGNED_IN, user: { ...SIGNED_IN.user, display_name: chosen } }),
      );
    });
    vi.stubGlobal("fetch", fetch);
    renderProbe();
    expect(
      await screen.findByRole("heading", { name: "Choose your display name" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("status")).not.toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "Display name" }), "betaporter");
    expect(screen.getByText("Public profile: /u/betaporter")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByTestId("user")).toHaveTextContent("betaporter");
    expect(fetch).toHaveBeenCalledWith(
      "/api/me",
      expect.objectContaining({
        method: "PATCH",
        body: JSON.stringify({ display_name: "betaporter" }),
      }),
    );
  });

  it("keeps profile setup open when a name is unavailable and refuses blank submission", async () => {
    const user = userEvent.setup();
    const fetch = vi.fn(async (_input: RequestInfo | URL, options?: RequestInit) =>
      options?.method === "PATCH"
        ? new Response(JSON.stringify({ code: "conflict", message: "That name is unavailable." }), {
            status: 409,
          })
        : new Response(
            JSON.stringify({ ...SIGNED_IN, user: { ...SIGNED_IN.user, display_name: null } }),
          ),
    );
    vi.stubGlobal("fetch", fetch);
    renderProbe();
    const name = await screen.findByRole("textbox", { name: "Display name" });
    fireEvent.submit(name.closest("form")!);
    expect(fetch).toHaveBeenCalledTimes(1);
    await user.type(name, "settings");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText("That name is unavailable.")).toBeInTheDocument();
    expect(name).toHaveValue("settings");
    expect(screen.queryByTestId("status")).not.toBeInTheDocument();
  });
  it("does not report a server failure as being signed out", async () => {
    stubMe(500);
    renderProbe();

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("failed");
    });

    // Distinct from `anonymous`, and it says why. A signed-in user whose
    // network blipped must not be told they have no account.
    expect(screen.getByTestId("error")).toHaveTextContent("Something went wrong.");
    expect(screen.getByTestId("error-status")).toHaveTextContent("500");
  });

  it("starts in the loading state rather than assuming either answer", async () => {
    stubMe(401);
    renderProbe();

    expect(screen.getByTestId("status")).toHaveTextContent("loading");

    // Settled before the test ends, so the state update belongs to this test
    // rather than leaking into the next one.
    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("anonymous");
    });
  });
});
