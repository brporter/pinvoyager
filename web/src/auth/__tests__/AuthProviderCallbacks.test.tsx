import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AuthProvider } from "../AuthProvider";
import { useAuth } from "../useAuth";

const mocks = vi.hoisted(() => ({
  refetch: vi.fn(),
  mutateAsync: vi.fn(),
  useMe: vi.fn(),
  useSignOut: vi.fn(),
}));

vi.mock("../../api/hooks", () => ({
  useMe: mocks.useMe,
  useSignOut: mocks.useSignOut,
}));

const signOutFailure = vi.fn();

function CallbackProbe() {
  const { refresh, signOut, status } = useAuth();
  return (
    <>
      <span data-testid="status">{status}</span>
      <button onClick={() => void refresh()}>Refresh session</button>
      <button onClick={() => void signOut().catch(signOutFailure)}>Sign out</button>
    </>
  );
}

function renderProbe() {
  return render(
    <AuthProvider>
      <CallbackProbe />
    </AuthProvider>,
  );
}

beforeEach(() => {
  signOutFailure.mockReset();
  mocks.refetch.mockReset().mockResolvedValue({});
  mocks.mutateAsync.mockReset().mockResolvedValue(undefined);
  mocks.useMe.mockReset().mockReturnValue({
    isPending: false,
    isError: false,
    data: null,
    error: null,
    refetch: mocks.refetch,
  });
  mocks.useSignOut.mockReset().mockReturnValue({ mutateAsync: mocks.mutateAsync });
});

describe("AuthProvider callbacks", () => {
  it("refetches the current-user query when refresh is requested", async () => {
    const user = userEvent.setup();
    renderProbe();

    await user.click(screen.getByRole("button", { name: "Refresh session" }));

    await waitFor(() => expect(mocks.refetch).toHaveBeenCalledOnce());
  });

  it("runs the sign-out mutation", async () => {
    const user = userEvent.setup();
    renderProbe();

    await user.click(screen.getByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(mocks.mutateAsync).toHaveBeenCalledOnce());
  });

  it("propagates a sign-out failure to the caller", async () => {
    const user = userEvent.setup();
    mocks.mutateAsync.mockRejectedValue(new Error("offline"));
    renderProbe();

    await user.click(screen.getByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(mocks.mutateAsync).toHaveBeenCalledOnce());
    expect(signOutFailure).toHaveBeenCalledWith(expect.objectContaining({ message: "offline" }));
  });
});
