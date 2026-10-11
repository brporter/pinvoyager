import "@testing-library/jest-dom/vitest";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ProviderButton } from "../ProviderButton";
import { PROVIDERS } from "../providers";

describe("ProviderButton", () => {
  it("gives all four providers the shared label grammar", () => {
    render(
      <>
        {PROVIDERS.map((provider) => (
          <ProviderButton key={provider} provider={provider} />
        ))}
      </>,
    );

    for (const name of ["Google", "Apple", "Microsoft", "Facebook"]) {
      expect(screen.getByRole("button", { name: `Continue with ${name}` })).toBeInTheDocument();
    }
  });

  it("starts the flow when activated", async () => {
    const onClick = vi.fn();
    render(<ProviderButton provider="google" onClick={onClick} />);

    await userEvent.click(screen.getByRole("button", { name: "Continue with Google" }));

    expect(onClick).toHaveBeenCalledOnce();
  });

  it("refuses a second activation once the redirect has started", async () => {
    // Two OAuth redirects race each other, and the loser is a confusing
    // callback with a state that no longer matches.
    const onClick = vi.fn();
    render(<ProviderButton provider="facebook" loading onClick={onClick} />);

    const button = screen.getByRole("button", { name: "Redirecting to Facebook…" });
    await userEvent.click(button);
    button.focus();
    await userEvent.keyboard("{Enter}");

    expect(onClick).not.toHaveBeenCalled();
    expect(button).toHaveAttribute("aria-busy", "true");
  });

  it("does not fire when disabled, as it is when already linked", async () => {
    const onClick = vi.fn();
    render(<ProviderButton provider="microsoft" disabled onClick={onClick} />);

    await userEvent.click(screen.getByRole("button", { name: "Continue with Microsoft" }));

    expect(onClick).not.toHaveBeenCalled();
  });

  it("keeps the provider logo out of the accessible name", () => {
    // The logos are mandated artwork, not labels. If one leaked into the name
    // the button would announce something like "Google Continue with Google".
    render(<ProviderButton provider="apple" />);

    expect(screen.getByRole("button")).toHaveAccessibleName("Continue with Apple");
  });

  it("is reachable by keyboard", async () => {
    render(<ProviderButton provider="google" />);

    await userEvent.tab();

    expect(screen.getByRole("button")).toHaveFocus();
  });
});
