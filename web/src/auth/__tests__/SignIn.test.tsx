import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AuthContext } from "../context";
import type { AuthState } from "../context";
import { SignIn } from "../SignIn";
import { LocationProbe } from "./support";

const signedOut: AuthState = {
  status: "anonymous",
  user: null,
  identities: [],
  error: null,
  refresh: vi.fn().mockResolvedValue(undefined),
  signOut: vi.fn().mockResolvedValue(undefined),
};

function renderAt(path: string, auth: AuthState = signedOut) {
  return render(
    <AuthContext.Provider value={auth}>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/signin" element={<SignIn />} />
          <Route path="*" element={<h1>Destination reached</h1>} />
        </Routes>
        <LocationProbe />
      </MemoryRouter>
    </AuthContext.Provider>,
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

describe("SignIn", () => {
  it.each(["anonymous", "loading", "failed"] as const)(
    "renders the provider choices while auth is %s",
    (status) => {
      renderAt(
        "/signin",
        status === "failed"
          ? { ...signedOut, status, error: new Error("read failed") }
          : { ...signedOut, status },
      );

      expect(
        screen.getByRole("heading", { name: "Sign in or create an account" }),
      ).toBeInTheDocument();
      expect(screen.getByText(/Voyages start private/)).toBeInTheDocument();
      for (const provider of ["Google", "Apple", "Microsoft"]) {
        expect(
          screen.getByRole("button", { name: `Continue with ${provider}` }),
        ).toBeInTheDocument();
      }
      expect(screen.queryByRole("button", { name: /Facebook/i })).not.toBeInTheDocument();
      expect(screen.queryByLabelText(/email|password/i)).not.toBeInTheDocument();
    },
  );

  it("redirects an authenticated visitor to the validated requested destination", async () => {
    const authenticated: AuthState = {
      ...signedOut,
      status: "authenticated",
      user: {
        id: "user-1",
        display_name: "Ada",
        avatar_url: null,
        is_site_admin: false,
        created_at: "2026-01-01T00:00:00Z",
      },
    };
    renderAt("/signin?redirect_to=%2Fsettings", authenticated);

    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent("/settings"));
    expect(screen.getByRole("heading", { name: "Destination reached" })).toBeInTheDocument();
  });

  it("starts one provider redirect and disables the competing choices", async () => {
    const user = userEvent.setup();
    const assign = vi.fn();
    const interceptedWindow = new Proxy(window, {
      get(target, property) {
        if (property === "location") return { assign };
        return Reflect.get(target, property, target);
      },
    });
    renderAt("/signin?redirect_to=%2Fupload");
    vi.stubGlobal("window", interceptedWindow);

    await user.click(screen.getByRole("button", { name: "Continue with Google" }));

    expect(assign).toHaveBeenCalledWith("/auth/google/start?redirect_to=%2Fupload");
    expect(screen.getByRole("button", { name: "Redirecting to Google…" })).toHaveAttribute(
      "aria-busy",
      "true",
    );
    expect(screen.getByRole("button", { name: "Continue with Apple" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Continue with Microsoft" })).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: "Continue with Facebook" }),
    ).not.toBeInTheDocument();
  });

  it("omits the development sign-in form when the production flag is false", () => {
    vi.stubEnv("DEV", false);
    renderAt("/signin");

    expect(screen.queryByRole("button", { name: "Forge a session" })).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Sign in or create an account" }),
    ).toBeInTheDocument();
  });

  it("refreshes the session after the development sign-in succeeds", async () => {
    const user = userEvent.setup();
    const refresh = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, status: 200 } as Response));
    renderAt("/signin", { ...signedOut, refresh });

    const submit = screen.getByRole("button", { name: "Forge a session" });
    await user.click(submit);

    await waitFor(() => expect(refresh).toHaveBeenCalledOnce());
  });
});
