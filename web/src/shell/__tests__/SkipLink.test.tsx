import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SkipLink } from "../SkipLink";

describe("SkipLink", () => {
  it("points to the configured main region and starts visually hidden", () => {
    render(<SkipLink targetId="page-main" />);

    const link = screen.getByRole("link", { name: "Skip to content" });
    expect(link).toHaveAttribute("href", "#page-main");
    expect(link).toHaveClass("pv-sr");
    expect(link).not.toHaveStyle({ position: "absolute" });
  });

  it("shows the link while focused and hides it again on blur", () => {
    render(<SkipLink targetId="main" />);
    const link = screen.getByRole("link", { name: "Skip to content" });

    fireEvent.focus(link);
    expect(link).not.toHaveClass("pv-sr");
    expect(link).toHaveStyle({ position: "absolute", zIndex: 100 });

    fireEvent.blur(link);
    expect(link).toHaveClass("pv-sr");
    expect(link).not.toHaveStyle({ position: "absolute" });
  });
});
