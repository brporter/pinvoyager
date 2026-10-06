import "@testing-library/jest-dom/vitest";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Badge } from "../Badge";
import { Filmstrip } from "../Filmstrip";
import { Photo } from "../Photo";
import { PhotoGrid } from "../PhotoGrid";

describe("Photo", () => {
  it("renders the photograph with its alternative text", () => {
    render(<Photo src="https://example.test/1.jpg" alt="A snowy street in Sapporo" />);

    expect(screen.getByRole("img", { name: "A snowy street in Sapporo" })).toBeInTheDocument();
  });

  it("keeps a caption-less photograph out of the accessibility tree", () => {
    // An empty alt is the right answer for a photo the surrounding page already
    // describes; "image" announced forty times is not.
    render(<Photo src="https://example.test/1.jpg" />);

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });

  it("shows nothing but the skeleton while loading", () => {
    const { container } = render(<Photo src="https://example.test/1.jpg" loading />);

    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector(".pv-skeleton")).not.toBeNull();
  });

  it("accounts for an unplaced photograph visibly rather than dropping it", () => {
    const { container } = render(
      <Photo src="https://example.test/1.jpg" unplaced badge={<Badge tone="unplaced">No location</Badge>} />,
    );

    expect(screen.getByText("No location")).toBeInTheDocument();
    expect(container.querySelector(".pv-photo")).toHaveAttribute("data-unplaced", "true");
  });

  it("is activatable by pointer and keyboard when it has a handler", async () => {
    const onSelect = vi.fn();
    render(<Photo label="Snowy street" onSelect={onSelect} gradient={1} />);

    const frame = screen.getByRole("button", { name: "Snowy street" });
    await userEvent.click(frame);
    frame.focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard(" ");

    expect(onSelect).toHaveBeenCalledTimes(3);
  });

  it("ignores keyboard keys other than Enter and Space", async () => {
    const onSelect = vi.fn();
    render(<Photo label="Snowy street" onSelect={onSelect} gradient={1} />);
    screen.getByRole("button", { name: "Snowy street" }).focus();

    await userEvent.keyboard("{ArrowDown}");

    expect(onSelect).not.toHaveBeenCalled();
  });

  it("reports its selected state when it is activatable", () => {
    render(<Photo label="Snowy street" selected onSelect={() => {}} gradient={1} />);

    expect(screen.getByRole("button", { name: "Snowy street" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("is not in the tab order when it is only a picture", async () => {
    render(
      <>
        <Photo src="https://example.test/1.jpg" alt="A snowy street" />
        <button type="button">After</button>
      </>,
    );

    await userEvent.tab();

    expect(screen.getByRole("button", { name: "After" })).toHaveFocus();
  });
});

describe("PhotoGrid", () => {
  it("names the group when asked to", () => {
    render(
      <PhotoGrid label="Photos in this voyage">
        <Photo gradient={1} />
        <Photo gradient={2} />
      </PhotoGrid>,
    );

    expect(screen.getByRole("group", { name: "Photos in this voyage" })).toBeInTheDocument();
  });
});

describe("Filmstrip", () => {
  it("can be focused so the arrow keys can scroll it", async () => {
    render(
      <Filmstrip label="Photos">
        <Photo ratio="1:1" gradient={2} />
        <Photo ratio="1:1" gradient={3} />
      </Filmstrip>,
    );

    await userEvent.tab();

    expect(screen.getByRole("group", { name: "Photos" })).toHaveFocus();
  });

  it("does not put an empty strip in the tab order", async () => {
    render(
      <>
        <Filmstrip label="Photos">{null}</Filmstrip>
        <button type="button">After</button>
      </>,
    );

    await userEvent.tab();

    expect(screen.getByRole("button", { name: "After" })).toHaveFocus();
  });

  it("omits the group role when no accessible label is provided", () => {
    const { container } = render(<Filmstrip><Photo ratio="1:1" gradient={2} /></Filmstrip>);

    expect(container.querySelector(".pv-filmstrip")).not.toHaveAttribute("role");
  });
});
