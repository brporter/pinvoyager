import "@testing-library/jest-dom/vitest";

import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Granularity } from "../../api/types";
import { GranularitySelector } from "../GranularitySelector";

const GUARANTEE = /Location data is removed from the files themselves/;

describe("GranularitySelector", () => {
  it("reports which precision is in force", () => {
    render(<GranularitySelector value="km10" onChange={() => {}} />);

    expect(screen.getByRole("button", { name: "~10 km" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Exact" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("reports the chosen step", async () => {
    const onChange = vi.fn();
    render(<GranularitySelector value="exact" onChange={onChange} />);

    await userEvent.click(screen.getByRole("button", { name: "~50 km" }));

    expect(onChange).toHaveBeenCalledWith<[Granularity]>("km50");
  });

  it("shows the consequence of the choice before publishing, not after", async () => {
    function Harness() {
      const [value, setValue] = useState<Granularity>("exact");
      return <GranularitySelector value={value} onChange={setValue} />;
    }
    render(<Harness />);

    // Exact draws a pin — the coordinate is claimed to be the real one.
    expect(
      screen.getByRole("img", { name: "A photograph at its exact recorded position" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/precise spot each was taken/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "~10 km" }));

    // Coarse draws an area, because a sharp pin at the centre of a rounding
    // makes a precision claim the data cannot support.
    expect(
      screen.queryByRole("img", { name: "A photograph at its exact recorded position" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "The area a viewer could infer at ~10 km" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/roughly the town or district, not the street/)).toBeInTheDocument();
  });

  it("states the stripping guarantee at every step", async () => {
    function Harness() {
      const [value, setValue] = useState<Granularity>("exact");
      return <GranularitySelector value={value} onChange={setValue} />;
    }
    render(<Harness />);

    // Without EXIF stripping the whole control is decorative, so the promise
    // is made where the choice is, at all four settings.
    expect(screen.getByText(GUARANTEE)).toBeInTheDocument();
    for (const step of ["~1 km", "~10 km", "~50 km"]) {
      await userEvent.click(screen.getByRole("button", { name: step }));
      expect(screen.getByText(GUARANTEE)).toBeInTheDocument();
    }
  });

  it("prefers the server's labels so the several places they appear cannot disagree", () => {
    render(
      <GranularitySelector
        value="km1"
        onChange={() => {}}
        labels={{ exact: "Exact", km1: "≈1 km", km10: "≈10 km", km50: "≈50 km" }}
      />,
    );

    expect(screen.getByRole("button", { name: "≈1 km" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("goes inert, with a reason, when there is nothing to coarsen", async () => {
    const onChange = vi.fn();
    render(
      <GranularitySelector
        value="km1"
        onChange={onChange}
        disabled
        disabledReason="None of the photos in this voyage have location data, so there is nothing to coarsen yet."
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "~50 km" }));

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText(/nothing to coarsen yet/)).toBeInTheDocument();
  });

  it("names the set of steps for a screen reader", () => {
    render(<GranularitySelector value="exact" onChange={() => {}} />);

    expect(screen.getByRole("group", { name: "Location precision" })).toBeInTheDocument();
  });

  it("reaches all four steps by keyboard", async () => {
    render(<GranularitySelector value="exact" onChange={() => {}} />);

    for (const step of ["Exact", "~1 km", "~10 km", "~50 km"]) {
      await userEvent.tab();
      expect(screen.getByRole("button", { name: step })).toHaveFocus();
    }
  });
});
