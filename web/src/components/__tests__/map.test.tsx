import "@testing-library/jest-dom/vitest";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Attribution, Cluster, MapArea, Pin } from "../MapPrimitives";

describe("Pin", () => {
  it("carries a name describing the photograph, not the marker", async () => {
    const onClick = vi.fn();
    render(<Pin label="Photo taken 14 February, 09:12" onClick={onClick} left="20%" top="30%" />);

    await userEvent.click(
      screen.getByRole("button", { name: "Photo taken 14 February, 09:12" }),
    );

    expect(onClick).toHaveBeenCalledOnce();
  });

  it("reports selection to assistive technology as well as visually", () => {
    render(<Pin label="Photo one" selected onClick={() => {}} />);

    const pin = screen.getByRole("button", { name: "Photo one" });
    expect(pin).toHaveAttribute("aria-pressed", "true");
    expect(pin).toHaveAttribute("data-selected", "true");
  });

  it("is a labeled graphic, not a dead tab stop, when it does nothing", async () => {
    render(
      <>
        <Pin label="A photograph at its exact recorded position" />
        <button type="button">After</button>
      </>,
    );

    expect(
      screen.getByRole("img", { name: "A photograph at its exact recorded position" }),
    ).toBeInTheDocument();

    await userEvent.tab();
    expect(screen.getByRole("button", { name: "After" })).toHaveFocus();
  });

  it("is reachable by keyboard when it is a control", async () => {
    render(<Pin label="Photo one" onClick={() => {}} />);

    await userEvent.tab();

    expect(screen.getByRole("button", { name: "Photo one" })).toHaveFocus();
  });
});

describe("Cluster", () => {
  it("says how many photographs it stands for", () => {
    render(<Cluster count={24} size="md" />);

    expect(screen.getByRole("img", { name: "24 photos here" })).toHaveTextContent("24");
  });

  it("becomes a keyboard-reachable control when it has a handler", async () => {
    const onClick = vi.fn();
    render(<Cluster count={156} size="lg" onClick={onClick} />);

    await userEvent.tab();
    const cluster = screen.getByRole("button", { name: "156 photos here" });
    expect(cluster).toHaveFocus();

    await userEvent.keyboard("{Enter}");
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe("MapArea", () => {
  it("names the region a viewer could infer when asked to", () => {
    render(<MapArea left="50%" top="52%" size={132} label="Photos within about 10 km" />);

    expect(screen.getByRole("img", { name: "Photos within about 10 km" })).toBeInTheDocument();
  });

  it("stays silent when a caption nearby already states the precision", () => {
    const { container } = render(<MapArea left="50%" top="52%" size={62} />);

    expect(container.querySelector(".pv-map-area")).toHaveAttribute("aria-hidden", "true");
  });

  it("draws the size it is given, because the radius is the precision claim", () => {
    const { container } = render(<MapArea left="50%" top="52%" size={230} />);

    const area = container.querySelector<HTMLElement>(".pv-map-area");
    expect(area?.style.width).toBe("230px");
    expect(area?.style.height).toBe("230px");
  });
});

describe("Attribution", () => {
  it("credits OpenStreetMap, which the ODbL license requires", () => {
    render(<Attribution />);

    expect(screen.getByRole("link", { name: "OpenStreetMap" })).toHaveAttribute(
      "href",
      "https://www.openstreetmap.org/copyright",
    );
    expect(screen.getByText(/contributors/)).toBeInTheDocument();
  });

  it("uses caller-provided attribution content and its dark style", () => {
    const { container } = render(
      <Attribution dark className="custom-attribution">
        Map data © contributors
      </Attribution>,
    );

    expect(screen.getByText("Map data © contributors")).toBeInTheDocument();
    expect(container.querySelector(".pv-attribution")).toHaveClass(
      "pv-attribution--dark",
      "custom-attribution",
    );
    expect(screen.queryByRole("link", { name: "OpenStreetMap" })).not.toBeInTheDocument();
  });
});
