import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AuthContext, type AuthState } from "../context";
import { useAuth } from "../useAuth";

function Probe() {
  const auth = useAuth();
  return <span>{auth.status}</span>;
}

describe("useAuth", () => {
  it("returns the state supplied by AuthProvider", () => {
    const state: AuthState = {
      status: "anonymous",
      user: null,
      identities: [],
      error: null,
      refresh: async () => {},
      signOut: async () => {},
    };

    render(
      <AuthContext.Provider value={state}>
        <Probe />
      </AuthContext.Provider>,
    );

    expect(screen.getByText("anonymous")).toBeInTheDocument();
  });

  it("throws when rendered outside AuthProvider", () => {
    expect(() => render(<Probe />)).toThrow(
      "useAuth must be used inside an <AuthProvider>",
    );
  });
});
