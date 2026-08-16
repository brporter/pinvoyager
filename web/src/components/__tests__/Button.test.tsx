import "@testing-library/jest-dom/vitest";

import type { FormEvent } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Button } from "../Button";
import { EditIcon } from "../icons";

describe("Button", () => {
  it("calls its handler when activated", async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Publish voyage</Button>);

    await userEvent.click(screen.getByRole("button", { name: "Publish voyage" }));

    expect(onClick).toHaveBeenCalledOnce();
  });

  it("does not call its handler when disabled", async () => {
    const onClick = vi.fn();
    render(
      <Button disabled onClick={onClick}>
        Publish voyage
      </Button>,
    );

    await userEvent.click(screen.getByRole("button", { name: "Publish voyage" }));

    expect(onClick).not.toHaveBeenCalled();
  });

  it("does not call its handler while loading, including from the keyboard", async () => {
    // The design system stops the mouse with `pointer-events: none`, which does
    // nothing about Enter on a focused button. This is the half that would be
    // missed.
    const onClick = vi.fn();
    render(
      <Button loading onClick={onClick}>
        Uploading…
      </Button>,
    );

    const button = screen.getByRole("button", { name: "Uploading…" });
    button.focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.click(button);

    expect(onClick).not.toHaveBeenCalled();
  });

  it("announces itself as busy while loading and keeps its label", () => {
    render(<Button loading>Uploading…</Button>);

    const button = screen.getByRole("button", { name: "Uploading…" });
    expect(button).toHaveAttribute("aria-busy", "true");
  });

  it("gives an icon-only button an accessible name", async () => {
    const onClick = vi.fn();
    render(
      <Button iconOnly aria-label="Edit voyage" onClick={onClick}>
        <EditIcon />
      </Button>,
    );

    await userEvent.click(screen.getByRole("button", { name: "Edit voyage" }));

    expect(onClick).toHaveBeenCalledOnce();
  });

  it("does not submit the form it happens to sit inside", async () => {
    const onSubmit = vi.fn((event: FormEvent) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button>Cancel</Button>
      </form>,
    );

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("submits when asked to", async () => {
    const onSubmit = vi.fn((event: FormEvent) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button type="submit" variant="primary">
          Save draft
        </Button>
      </form>,
    );

    await userEvent.click(screen.getByRole("button", { name: "Save draft" }));

    expect(onSubmit).toHaveBeenCalledOnce();
  });

  it("is reachable by keyboard", async () => {
    render(<Button>Publish voyage</Button>);

    await userEvent.tab();

    expect(screen.getByRole("button", { name: "Publish voyage" })).toHaveFocus();
  });
});
