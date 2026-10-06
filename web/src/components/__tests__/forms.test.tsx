import "@testing-library/jest-dom/vitest";

import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Checkbox } from "../Checkbox";
import { Select } from "../Select";
import { Switch } from "../Switch";
import { TextArea } from "../TextArea";
import { TextField } from "../TextField";

describe("TextField", () => {
  it("associates its label with the input", () => {
    render(<TextField label="Voyage title" />);

    expect(screen.getByLabelText("Voyage title")).toBeInstanceOf(HTMLInputElement);
  });

  it("associates its error message with the input", () => {
    // The point of the wiring: a screen reader reads the failure on focus
    // rather than leaving someone to discover it by trial.
    render(
      <TextField label="Voyage title" required error="A title is required before publishing." />,
    );

    const input = screen.getByLabelText(/Voyage title/);
    expect(input).toBeInvalid();
    expect(input).toHaveAccessibleDescription("A title is required before publishing.");
  });

  it("associates its help text with the input when there is no error", () => {
    render(<TextField label="Voyage title" help="Shown publicly on your profile." />);

    const input = screen.getByLabelText("Voyage title");
    expect(input).toHaveAccessibleDescription("Shown publicly on your profile.");
    expect(input).not.toBeInvalid();
  });

  it("marks a required field required without announcing the asterisk", () => {
    render(<TextField label="Voyage title" required />);

    const input = screen.getByLabelText(/Voyage title/);
    expect(input).toBeRequired();
    expect(input).toHaveAccessibleName("Voyage title");
  });

  it("keeps two fields' errors apart when both are on screen", () => {
    render(
      <>
        <TextField label="Voyage title" error="A title is required." />
        <TextField label="Location name" error="Unknown place." />
      </>,
    );

    expect(screen.getByLabelText("Voyage title")).toHaveAccessibleDescription(
      "A title is required.",
    );
    expect(screen.getByLabelText("Location name")).toHaveAccessibleDescription(
      "Unknown place.",
    );
  });

  it("accepts typing", async () => {
    render(<TextField label="Voyage title" defaultValue="" />);

    const input = screen.getByLabelText("Voyage title");
    await userEvent.type(input, "Hokkaido");

    expect(input).toHaveValue("Hokkaido");
  });

  it("cannot be typed into when disabled", async () => {
    render(<TextField label="Voyage title" disabled defaultValue="Three weeks in Hokkaido" />);

    const input = screen.getByLabelText("Voyage title");
    await userEvent.type(input, "more");

    expect(input).toHaveValue("Three weeks in Hokkaido");
  });
});

describe("TextArea", () => {
  it("associates its error message with the control", () => {
    render(<TextArea label="Description" error="Too long." />);

    const field = screen.getByLabelText("Description");
    expect(field).toBeInstanceOf(HTMLTextAreaElement);
    expect(field).toBeInvalid();
    expect(field).toHaveAccessibleDescription("Too long.");
  });
});

describe("Select", () => {
  it("reports the chosen option", async () => {
    const onChange = vi.fn();
    render(
      <Select label="Photo order" defaultValue="oldest" onChange={onChange}>
        <option value="oldest">Capture time — oldest first</option>
        <option value="newest">Capture time — newest first</option>
        <option value="manual">Manual</option>
      </Select>,
    );

    const select = screen.getByLabelText("Photo order");
    await userEvent.selectOptions(select, "manual");

    expect(select).toHaveValue("manual");
    expect(onChange).toHaveBeenCalled();
  });

  it("associates its error message with the control", () => {
    render(
      <Select label="Photo order" error="Pick an order.">
        <option value="oldest">Oldest first</option>
      </Select>,
    );

    expect(screen.getByLabelText("Photo order")).toHaveAccessibleDescription("Pick an order.");
  });
});

describe("Checkbox", () => {
  it("reports its state to assistive technology and toggles from the keyboard", async () => {
    render(<Checkbox>Include photos without location data</Checkbox>);

    const box = screen.getByRole("checkbox", {
      name: "Include photos without location data",
    });
    expect(box).not.toBeChecked();

    box.focus();
    await userEvent.keyboard(" ");

    expect(box).toBeChecked();
  });

  it("honors a controlled `checked` prop", async () => {
    function Controlled() {
      const [on, setOn] = useState(false);
      return (
        <Checkbox checked={on} onChange={(event) => setOn(event.target.checked)}>
          Include photos without location data
        </Checkbox>
      );
    }
    render(<Controlled />);

    const box = screen.getByRole("checkbox");
    await userEvent.click(box);
    expect(box).toBeChecked();

    await userEvent.click(box);
    expect(box).not.toBeChecked();
  });

  it("does not toggle when disabled", async () => {
    const onChange = vi.fn();
    render(
      <Checkbox disabled onChange={onChange}>
        Include photos without location data
      </Checkbox>,
    );

    await userEvent.click(screen.getByRole("checkbox"));

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("checkbox")).not.toBeChecked();
  });
});

describe("Switch", () => {
  it("reports its state to assistive technology", () => {
    const { rerender } = render(
      <Switch checked={false} onCheckedChange={() => {}} label="Publish voyage" />,
    );

    expect(screen.getByRole("switch", { name: "Publish voyage" })).not.toBeChecked();

    rerender(<Switch checked onCheckedChange={() => {}} label="Publish voyage" />);

    expect(screen.getByRole("switch", { name: "Publish voyage" })).toBeChecked();
  });

  it("toggles on click, Space and Enter", async () => {
    const onCheckedChange = vi.fn();
    render(
      <Switch checked={false} onCheckedChange={onCheckedChange} label="Publish voyage" />,
    );

    const control = screen.getByRole("switch");
    await userEvent.click(control);
    control.focus();
    await userEvent.keyboard(" ");
    await userEvent.keyboard("{Enter}");

    expect(onCheckedChange).toHaveBeenCalledTimes(3);
    expect(onCheckedChange).toHaveBeenLastCalledWith(true);
  });

  it("ignores keyboard keys other than Space and Enter", async () => {
    const onCheckedChange = vi.fn();
    render(<Switch checked={false} onCheckedChange={onCheckedChange} label="Publish voyage" />);
    screen.getByRole("switch", { name: "Publish voyage" }).focus();

    await userEvent.keyboard("{ArrowDown}");

    expect(onCheckedChange).not.toHaveBeenCalled();
  });

  it("is reachable by keyboard", async () => {
    render(<Switch checked={false} onCheckedChange={() => {}} label="Publish voyage" />);

    await userEvent.tab();

    expect(screen.getByRole("switch")).toHaveFocus();
  });

  it("does not toggle, and leaves the tab order, when disabled", async () => {
    const onCheckedChange = vi.fn();
    render(
      <Switch
        checked
        disabled
        onCheckedChange={onCheckedChange}
        label="Publish voyage"
      />,
    );

    const control = screen.getByRole("switch");
    await userEvent.click(control);

    expect(onCheckedChange).not.toHaveBeenCalled();
    expect(control).toHaveAttribute("tabindex", "-1");
  });

  it("shows the state in text as well as in the control", () => {
    render(
      <Switch
        checked
        onCheckedChange={() => {}}
        label="Publish voyage"
        stateText="Published — visible to everyone"
      />,
    );

    expect(screen.getByText("Published — visible to everyone")).toBeInTheDocument();
  });
});
