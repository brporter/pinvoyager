/**
 * Route protection, exercised through the real route table in `App.tsx` rather
 * than through a tree assembled for the test.
 *
 * That matters here: the thing most likely to break is not `RequireAuth` itself
 * but which routes are wrapped in it, and a test that builds its own routes
 * would keep passing after someone moved `/settings` outside the guard.
 */

import { render, screen, waitFor } from "@testing-library/react";
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

describe("protected routes, signed out", () => {
  it("redirects /upload to sign-in and preserves where the user was going", async () => {
    stubMe(401);
    renderAt("/upload");

    await waitFor(() => {
      expect(location()).toBe("/signin?redirect_to=%2Fupload");
    });

    // And the sign-in screen is actually what rendered, not just the URL.
    expect(
      screen.getByRole("heading", { name: "Sign in or create an account" }),
    ).toBeInTheDocument();
  });

  it("redirects /settings the same way", async () => {
    stubMe(401);
    renderAt("/settings");

    await waitFor(() => {
      expect(location()).toBe("/signin?redirect_to=%2Fsettings");
    });
  });

  it("redirects /voyages/:id/edit and keeps the voyage id in the destination", async () => {
    stubMe(401);
    renderAt("/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f/edit");

    await waitFor(() => {
      expect(location()).toBe(
        "/signin?redirect_to=%2Fvoyages%2F0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f%2Fedit",
      );
    });
  });

  it("does not redirect before /api/me has answered", () => {
    stubMe(401);
    renderAt("/upload");

    // Still on the protected route while the read is in flight. Redirecting
    // here would bounce every signed-in user off every protected route on
    // reload and back again, which looks like the session having been lost.
    expect(location()).toBe("/upload");
    expect(screen.getByText("Checking your session…")).toBeInTheDocument();
  });

  it("leaves the public routes alone", async () => {
    stubMe(401);
    renderAt("/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f");

    // Asserted on the address, not on anything the screen rendered. What
    // RequireAuth does is decide whether to redirect, and a public route is
    // one it must leave alone; which heading the voyage screen happens to
    // show while its data loads is that screen's business. An earlier version
    // of this test looked for the routing placeholder's heading and broke the
    // moment the real screen replaced it, without anything being wrong.
    await waitFor(() => {
      expect(screen.queryByText("Checking your session…")).not.toBeInTheDocument();
    });

    expect(location()).toBe("/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f");
  });
});

describe("protected routes, signed in", () => {
  it("renders the route", async () => {
    stubMe(200, SIGNED_IN);
    renderAt("/upload");

    // Same reasoning as above: the property under test is that a signed-in
    // user is not sent to /signin and the session check has finished. What the
    // upload screen draws afterwards belongs to the upload screen's own tests.
    await waitFor(() => {
      expect(screen.queryByText("Checking your session…")).not.toBeInTheDocument();
    });

    expect(location()).toBe("/upload");
  });

  it("sends someone who reaches /signin anyway on to their destination", async () => {
    stubMe(200, SIGNED_IN);
    renderAt("/signin?redirect_to=%2Fsettings");

    await waitFor(() => {
      expect(location()).toBe("/settings");
    });
  });
});

describe("the redirect target is validated, not merely carried", () => {
  it("refuses an off-site destination handed to the sign-in screen", async () => {
    stubMe(200, SIGNED_IN);
    // A signed-in user following a crafted link. Honouring this would send the
    // browser to another origin from a URL that showed our domain the whole
    // way — which is exactly what makes a sign-in link worth forging.
    renderAt("/signin?redirect_to=https%3A%2F%2Fevil.example%2F");

    await waitFor(() => {
      expect(location()).toBe("/");
    });
  });

  it("refuses a protocol-relative destination", async () => {
    stubMe(200, SIGNED_IN);
    renderAt("/signin?redirect_to=%2F%2Fevil.example%2F");

    await waitFor(() => {
      expect(location()).toBe("/");
    });
  });

  it("refuses an on-site path that is not on the allowlist", async () => {
    stubMe(200, SIGNED_IN);
    // On-site is not by itself enough — the server's allowlist would drop this
    // too, and disagreeing with it would strand the user with no explanation.
    renderAt("/signin?redirect_to=%2Fadmin");

    await waitFor(() => {
      expect(location()).toBe("/");
    });
  });

  it("honours a destination that does pass", async () => {
    stubMe(200, SIGNED_IN);
    renderAt("/signin?redirect_to=%2Fvoyages%2Fabc%2Fedit");

    await waitFor(() => {
      expect(location()).toBe("/voyages/abc/edit");
    });
  });
});
