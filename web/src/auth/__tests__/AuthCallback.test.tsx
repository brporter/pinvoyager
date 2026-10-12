/**
 * Which of the four callback states is chosen, and what the conflict one says.
 *
 * The copy is asserted rather than eyeballed for one sentence only: "Nothing
 * has been changed on either account" is a promise about server state that the
 * link handler is written to keep, and a user whose link attempt failed is
 * afraid they have half-merged something. It is the one line on this screen
 * that must not quietly drift.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { App } from "../../App";
import { LocationProbe, SIGNED_IN, WithQueryClient, stubMe } from "./support";

function renderAt(path: string) {
  return render(
    <WithQueryClient>
      <MemoryRouter initialEntries={[path]}>
        <App />
        <LocationProbe />
      </MemoryRouter>
    </WithQueryClient>,
  );
}

const location = () => screen.getByTestId("location").textContent;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("/auth/callback", () => {
  it("shows the conflict state with the promise the server keeps", async () => {
    // A conflict arrives in a browser that *is* signed in — linking a second
    // provider happens while signed in — so this also covers the case where a
    // valid session must not cause the explanation to be skipped.
    stubMe(200, SIGNED_IN);
    renderAt("/auth/callback?status=conflict&provider=apple");

    expect(
      screen.getByRole("heading", { name: "That Apple ID is already in use" }),
    ).toBeInTheDocument();

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Nothing has been changed on either account.",
    );

    // Still on the callback: an outcome named in the URL is never overridden
    // by the session.
    await waitFor(() => {
      expect(location()).toBe("/auth/callback?status=conflict&provider=apple");
    });
  });

  it("names the provider in the cancellation copy and offers a way back", () => {
    stubMe(401);
    renderAt("/auth/callback?status=canceled&provider=google");

    expect(screen.getByRole("heading", { name: "Sign-in canceled" })).toBeInTheDocument();
    expect(
      screen.getByText(/You declined access at Google, so nothing was created\./),
    ).toBeInTheDocument();
  });

  it("returns to sign-in with the validated destination after cancellation", async () => {
    stubMe(401);
    renderAt("/auth/callback?status=cancelled&redirect_to=%2Fvoyages%2Fv1");

    await userEvent.click(screen.getByRole("button", { name: "Back to sign in" }));

    await waitFor(() => {
      expect(location()).toBe("/signin?redirect_to=%2Fvoyages%2Fv1");
    });
  });

  it("shows one generic failure and quotes the reference", () => {
    stubMe(401);
    renderAt("/auth/callback?status=bad_signature&provider=microsoft&ref=7f3a-2b91");

    // Not "bad signature". Every failure reaches the user as the same sentence;
    // the real cause is in the server log against this reference.
    expect(
      screen.getByRole("heading", { name: "We could not complete sign-in" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Reference 7f3a-2b91")).toBeInTheDocument();
  });

  it("retries a failed callback at sign-in while preserving its safe destination", async () => {
    stubMe(401);
    renderAt("/auth/callback?status=failed&redirect_to=%2Fsettings");

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));

    await waitFor(() => {
      expect(location()).toBe("/signin?redirect_to=%2Fsettings");
    });
  });

  it("verifies while the session read is in flight rather than declaring failure", () => {
    stubMe(401);
    renderAt("/auth/callback?provider=google");

    expect(screen.getByRole("heading", { name: "Signing you in…" })).toBeInTheDocument();
    expect(screen.getByText("Verifying your Google account.")).toBeInTheDocument();
  });

  it("accepts pending as the explicit verifying outcome", () => {
    stubMe(401);
    renderAt("/auth/callback?status=pending&provider=unknown");

    expect(screen.getByRole("heading", { name: "Signing you in…" })).toBeInTheDocument();
    expect(screen.getByText("Verifying your account.")).toBeInTheDocument();
  });

  it("restarts the conflicting provider and lets a signed-in user cancel to settings", async () => {
    stubMe(200, SIGNED_IN);
    const assign = vi.fn();
    vi.stubGlobal("location", { assign });
    renderAt("/auth/callback?status=already_linked&provider=apple");

    await userEvent.click(screen.getByRole("button", { name: "Sign in with Apple instead" }));
    expect(assign).toHaveBeenCalledWith("/auth/apple/start");

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(location()).toBe("/settings"));
  });

  it.each(["not-a-provider", "facebook"])("does not offer a restart for %s", (provider) => {
    stubMe(401);
    renderAt(`/auth/callback?status=conflict&provider=${provider}`);

    expect(screen.queryByRole("button", { name: /Sign in with/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("sends an anonymous user home when they cancel a provider conflict", async () => {
    stubMe(401);
    renderAt("/auth/callback?status=conflict&provider=apple");

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(location()).toBe("/"));
  });

  it("falls through to the failure state once the session says nobody is signed in", async () => {
    stubMe(401);
    renderAt("/auth/callback?provider=google");

    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "We could not complete sign-in" }),
      ).toBeInTheDocument();
    });
  });

  it("sends a signed-in visitor onward to a destination that passes validation", async () => {
    stubMe(200, SIGNED_IN);
    renderAt("/auth/callback?redirect_to=%2Fsettings");

    await waitFor(() => {
      expect(location()).toBe("/settings");
    });
  });

  it("refuses an off-site destination on the callback too", async () => {
    stubMe(200, SIGNED_IN);
    renderAt("/auth/callback?redirect_to=https%3A%2F%2Fevil.example%2F");

    await waitFor(() => {
      expect(location()).toBe("/");
    });
  });
});
