import "@testing-library/jest-dom/vitest";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Badge } from "../Badge";
import { Button } from "../Button";
import { EmptyState } from "../EmptyState";
import { Skeleton } from "../Skeleton";
import { Toast } from "../Toast";
import { PinIcon } from "../icons";

describe("Badge", () => {
  it("says which side of the publish line a voyage is on", () => {
    render(
      <>
        <Badge tone="draft">Draft</Badge>
        <Badge tone="public">Public</Badge>
      </>,
    );

    expect(screen.getByText("Draft")).toBeInTheDocument();
    expect(screen.getByText("Public")).toBeInTheDocument();
  });
});

describe("Toast", () => {
  it("uses the polite info role and default tone when none is supplied", () => {
    const { container } = render(<Toast title="Photos saved" />);

    expect(screen.getByRole("status")).toHaveTextContent("Photos saved");
    expect(container.querySelector(".pv-toast")).not.toHaveClass("pv-toast--info");
  });

  it("announces a failure by interrupting", () => {
    // A failed upload the user does not hear about is a failed upload they
    // will not retry.
    render(
      <Toast tone="error" title="2 photos could not be uploaded">
        One was not an image; one exceeded 25 MB.
      </Toast>,
    );

    const toast = screen.getByRole("alert");
    expect(toast).toHaveTextContent("2 photos could not be uploaded");
    expect(toast).toHaveTextContent("One was not an image; one exceeded 25 MB.");
  });

  it("announces a warning by interrupting too", () => {
    render(<Toast tone="warning" title="Your session expired" />);

    expect(screen.getByRole("alert")).toHaveTextContent("Your session expired");
  });

  it("announces a success politely", () => {
    render(<Toast tone="success" title="Voyage published" />);

    expect(screen.getByRole("status")).toHaveTextContent("Voyage published");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("can be dismissed from the keyboard, with a named control", async () => {
    const onDismiss = vi.fn();
    render(<Toast tone="success" title="Voyage published" onDismiss={onDismiss} />);

    await userEvent.tab();
    expect(screen.getByRole("button", { name: "Dismiss" })).toHaveFocus();

    await userEvent.keyboard("{Enter}");
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("has no dismiss control when it cannot be dismissed", () => {
    render(<Toast tone="success" title="Voyage published" />);

    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

describe("EmptyState", () => {
  it("explains what happened and offers exactly one way forward", async () => {
    const onClick = vi.fn();
    render(
      <EmptyState
        icon={<PinIcon size={40} strokeWidth={1.5} />}
        title="No voyages yet"
        action={
          <Button variant="primary" onClick={onClick}>
            Upload photos
          </Button>
        }
      >
        A voyage is a set of photos on a map.
      </EmptyState>,
    );

    expect(screen.getByRole("heading", { name: "No voyages yet" })).toBeInTheDocument();
    expect(screen.getByText("A voyage is a set of photos on a map.")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Upload photos" }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("quotes a reference on the error variant and nothing else", () => {
    render(
      <EmptyState
        tone="danger"
        icon={<PinIcon size={40} />}
        title="This voyage could not be loaded"
      >
        Something went wrong on our end. Reference <code>7f3a-2b91</code> if you report it.
      </EmptyState>,
    );

    expect(screen.getByText("7f3a-2b91")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "This voyage could not be loaded" }),
    ).toBeInTheDocument();
  });
});

describe("Skeleton", () => {
  it("stays out of the accessibility tree", () => {
    // It represents the absence of content. Reading out a row of empty boxes
    // is worse than silence; the region above it carries `aria-busy`.
    const { container } = render(<Skeleton width="60%" height={22} />);

    expect(container.querySelector(".pv-skeleton")).toHaveAttribute("aria-hidden", "true");
  });

  it("takes the shape of what is coming so the layout does not jump", () => {
    const { container } = render(<Skeleton width="60%" height={22} />);

    const element = container.querySelector<HTMLElement>(".pv-skeleton");
    expect(element?.style.width).toBe("60%");
    expect(element?.style.height).toBe("22px");
  });
});
