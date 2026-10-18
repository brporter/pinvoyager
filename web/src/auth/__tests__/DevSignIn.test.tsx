import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DevSignIn } from "../DevSignIn";

afterEach(() => {
  vi.unstubAllGlobals();
});

function response(status: number, ok = status >= 200 && status < 300): Response {
  return { status, ok } as Response;
}

describe("DevSignIn", () => {
  it("posts the entered subject and calls back after a successful response", async () => {
    const user = userEvent.setup();
    const fetch = vi.fn().mockResolvedValue(response(200));
    const onSignedIn = vi.fn();
    vi.stubGlobal("fetch", fetch);
    render(<DevSignIn onSignedIn={onSignedIn} />);

    const subject = screen.getByRole("textbox", { name: "Sign in as" });
    await user.clear(subject);
    await user.type(subject, "traveler@example.test");
    await user.click(screen.getByRole("button", { name: "Forge a session" }));

    await waitFor(() => expect(onSignedIn).toHaveBeenCalledOnce());
    expect(fetch).toHaveBeenCalledWith("/auth/dev/login", {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ subject: "traveler@example.test", grant_admin: false }),
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("optionally requests administrator privileges", async () => {
    const user = userEvent.setup();
    const fetch = vi.fn().mockResolvedValue(response(200));
    vi.stubGlobal("fetch", fetch);
    render(<DevSignIn onSignedIn={vi.fn()} />);
    const checkbox = screen.getByRole("checkbox", { name: "Grant site administrator privileges" });
    expect(checkbox).not.toBeChecked();
    await user.click(checkbox);
    await user.click(screen.getByRole("button", { name: "Forge a session" }));
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({
      subject: "dev@example.com",
      grant_admin: true,
    });
  });

  it("explains when the development endpoint is unavailable", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response(404, false)));
    render(<DevSignIn onSignedIn={vi.fn()} />);

    fireEvent.submit(screen.getByRole("button", { name: "Forge a session" }).closest("form")!);

    expect(await screen.findByText(/The development sign-in is not available/)).toBeInTheDocument();
    expect(screen.getByLabelText("Sign in as")).toHaveAttribute("aria-invalid", "true");
  });

  it.each([400, 500])("reports an unsuccessful HTTP response (%i)", async (status) => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response(status, false)));
    render(<DevSignIn onSignedIn={vi.fn()} />);

    fireEvent.submit(screen.getByRole("button", { name: "Forge a session" }).closest("form")!);

    expect(
      await screen.findByText(`Development sign-in failed (${String(status)}).`),
    ).toBeInTheDocument();
  });

  it("reports a network failure and clears it on the next attempt", async () => {
    const fetch = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("offline"))
      .mockResolvedValueOnce(response(200));
    const onSignedIn = vi.fn();
    vi.stubGlobal("fetch", fetch);
    render(<DevSignIn onSignedIn={onSignedIn} />);
    const form = screen.getByRole("button", { name: "Forge a session" }).closest("form")!;

    fireEvent.submit(form);
    expect(
      await screen.findByText("Could not reach the API. Is it running on :8080?"),
    ).toBeInTheDocument();

    fireEvent.submit(form);
    await waitFor(() => expect(onSignedIn).toHaveBeenCalledOnce());
    expect(
      screen.queryByText("Could not reach the API. Is it running on :8080?"),
    ).not.toBeInTheDocument();
  });

  it("disables submission for a blank or whitespace-only subject", async () => {
    const user = userEvent.setup();
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    render(<DevSignIn onSignedIn={vi.fn()} />);
    const subject = screen.getByRole("textbox", { name: "Sign in as" });
    const submit = screen.getByRole("button", { name: "Forge a session" });

    await user.clear(subject);
    await user.type(subject, "   ");
    expect(submit).toBeDisabled();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("shows the pending state until the sign-in request resolves", async () => {
    let resolveFetch!: (value: Response) => void;
    const fetch = vi.fn().mockReturnValue(
      new Promise<Response>((resolve) => {
        resolveFetch = resolve;
      }),
    );
    vi.stubGlobal("fetch", fetch);
    render(<DevSignIn onSignedIn={vi.fn()} />);
    const submit = screen.getByRole("button", { name: "Forge a session" });

    fireEvent.submit(submit.closest("form")!);
    await waitFor(() => expect(submit).toHaveAttribute("aria-busy", "true"));

    resolveFetch(response(200));
    await waitFor(() => expect(submit).not.toHaveAttribute("aria-busy"));
  });

  it("reports a rejected sign-in callback as an API failure", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response(200)));
    render(<DevSignIn onSignedIn={() => Promise.reject(new Error("refresh failed"))} />);

    fireEvent.submit(screen.getByRole("button", { name: "Forge a session" }).closest("form")!);

    expect(
      await screen.findByText("Could not reach the API. Is it running on :8080?"),
    ).toBeInTheDocument();
  });
});
