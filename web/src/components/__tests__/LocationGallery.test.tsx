import { StrictMode } from "react";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import type { VoyagePhoto } from "../../api/types";
import { LocationGallery } from "../LocationGallery";

const photo = (
  id: string,
  variant_status: VoyagePhoto["variant_status"] = "ready",
): VoyagePhoto => ({
  id,
  position: 1,
  width: 800,
  height: 600,
  captured_at: null,
  variant_status,
  thumbnail_url: `/api/photos/${id}/thumb`,
  display_url: `/api/photos/${id}/display`,
  full_url: `/api/photos/${id}/full`,
  placed: true,
  coordinate: { latitude: 43, longitude: 141, precision_m: 1000 },
});

it("uses a native modal, labeled heading, figures and a dialog close form", () => {
  const onClose = vi.fn();
  render(<LocationGallery photos={[photo("a")]} onClose={onClose} />);
  const dialog = screen.getByRole("dialog", { name: "Photos at this location" });
  expect(dialog.tagName).toBe("DIALOG");
  expect(HTMLDialogElement.prototype.showModal).toHaveBeenCalled();
  expect(within(dialog).getByText("1 of 1")).toBeInTheDocument();
  expect(within(dialog).queryByText(/~1 km/)).not.toBeInTheDocument();
  expect(
    within(dialog).getByRole("button", { name: "Close gallery" }).closest("form"),
  ).toHaveAttribute("method", "dialog");
  expect(within(dialog).getByRole("img")).toHaveAttribute("src", "/api/photos/a/display");
  act(() => (dialog as HTMLDialogElement).close());
  expect(onClose).toHaveBeenCalledTimes(1);
});

it("shows pending and failed photos explicitly and updates when processing finishes", () => {
  const view = render(
    <LocationGallery photos={[photo("a", "pending"), photo("b", "failed")]} onClose={() => {}} />,
  );
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  expect(screen.getByText("Processing photo…")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Next photo" }));
  expect(screen.getByText("This photo could not be processed.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Previous photo" }));
  view.rerender(<LocationGallery photos={[photo("a"), photo("b", "failed")]} onClose={() => {}} />);
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/a/display");
  expect(screen.queryByText("Processing photo…")).not.toBeInTheDocument();
});

it("allows the native Escape cancellation and closes an open dialog on unmount", () => {
  const view = render(<LocationGallery photos={[photo("a")]} onClose={() => {}} />);
  const cancel = new Event("cancel", { cancelable: true });
  expect(screen.getByRole("dialog").dispatchEvent(cancel)).toBe(true);
  expect(cancel.defaultPrevented).toBe(false);
  view.unmount();
  expect(HTMLDialogElement.prototype.close).toHaveBeenCalled();
});

it("does not dismiss itself when React StrictMode replays its effect", () => {
  const onClose = vi.fn();
  render(
    <StrictMode>
      <LocationGallery photos={[photo("a")]} onClose={onClose} />
    </StrictMode>,
  );
  expect(screen.getByRole("dialog")).toHaveAttribute("open");
  expect(onClose).not.toHaveBeenCalled();
});

it("ignores a queued close event after the dialog has already reopened", () => {
  const onClose = vi.fn();
  render(<LocationGallery photos={[photo("a")]} onClose={onClose} />);
  fireEvent(screen.getByRole("dialog"), new Event("close"));
  expect(onClose).not.toHaveBeenCalled();
});

it("shows one photo at a time, with accessible arrows and an icon-only close action", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  render(<LocationGallery photos={[photo("a"), photo("b"), photo("c")]} onClose={onClose} />);
  expect(screen.getAllByRole("img")).toHaveLength(1);
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/a/display");
  expect(screen.getByRole("button", { name: "Previous photo" })).toBeDisabled();
  expect(screen.getByText("1 of 3")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Next photo" }));
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/b/display");
  await user.click(screen.getByRole("button", { name: "Next photo" }));
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/c/display");
  expect(screen.getByText("3 of 3")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next photo" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Previous photo" }));
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/b/display");
  const close = screen.getByRole("button", { name: "Close gallery" });
  expect(close).toHaveTextContent("");
  expect(close.querySelector("svg")).not.toBeNull();
  await user.click(close);
  expect(onClose).toHaveBeenCalledTimes(1);
});

it("supports arrow keys, respects boundaries and leaves other keys alone", () => {
  render(<LocationGallery photos={[photo("a"), photo("b")]} onClose={() => {}} />);
  const dialog = screen.getByRole("dialog");
  fireEvent.keyDown(dialog, { key: "ArrowLeft" });
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/a/display");
  fireEvent.keyDown(dialog, { key: "ArrowRight" });
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/b/display");
  fireEvent.keyDown(dialog, { key: "ArrowRight" });
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/b/display");
  fireEvent.keyDown(dialog, { key: "ArrowLeft" });
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/a/display");
  const unrelated = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
  expect(dialog.dispatchEvent(unrelated)).toBe(true);
});

it("keeps the active photo during refresh and falls back if it is removed", () => {
  const props = { onClose: () => {} };
  const view = render(<LocationGallery {...props} photos={[photo("a"), photo("b")]} />);
  fireEvent.click(screen.getByRole("button", { name: "Next photo" }));
  view.rerender(<LocationGallery {...props} photos={[photo("c"), photo("a"), photo("b")]} />);
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/b/display");
  expect(screen.getByText("3 of 3")).toBeInTheDocument();
  view.rerender(<LocationGallery {...props} photos={[photo("c")]} />);
  expect(screen.getByRole("img")).toHaveAttribute("src", "/api/photos/c/display");
  view.rerender(<LocationGallery {...props} photos={[]} />);
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  expect(screen.getByText("0 photos")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Previous photo" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Next photo" })).toBeDisabled();
});

function pointer(dialog: HTMLElement, type: string, x: number, y: number, button = 0) {
  fireEvent(dialog, new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button }));
}
function bounds(dialog: HTMLElement) {
  vi.spyOn(dialog, "getBoundingClientRect").mockReturnValue({
    left: 100,
    top: 100,
    right: 500,
    bottom: 500,
  } as DOMRect);
}

it.each([
  [50, 200],
  [550, 200],
  [200, 50],
  [200, 550],
])("dismisses a backdrop click outside the dialog at (%s,%s)", (x, y) => {
  const onClose = vi.fn();
  render(<LocationGallery photos={[photo("a")]} onClose={onClose} />);
  const dialog = screen.getByRole("dialog");
  expect(dialog).toHaveAttribute("closedby", "any");
  bounds(dialog);
  pointer(dialog, "pointerdown", x, y);
  pointer(dialog, "click", x, y);
  expect(onClose).toHaveBeenCalledTimes(1);
});

it("does not dismiss for content, padding, an inside-to-outside drag or a secondary click", () => {
  const onClose = vi.fn();
  render(<LocationGallery photos={[photo("a")]} onClose={onClose} />);
  const dialog = screen.getByRole("dialog");
  bounds(dialog);
  pointer(screen.getByRole("img"), "pointerdown", 200, 200);
  pointer(screen.getByRole("img"), "click", 200, 200);
  pointer(dialog, "pointerdown", 200, 200);
  pointer(dialog, "click", 200, 200);
  pointer(dialog, "pointerdown", 200, 200);
  pointer(dialog, "click", 50, 200);
  pointer(dialog, "pointerdown", 50, 200);
  pointer(dialog, "click", 200, 200);
  pointer(dialog, "pointerdown", 50, 200, 2);
  pointer(dialog, "click", 50, 200, 2);
  expect(onClose).not.toHaveBeenCalled();
});

it("opens an empty gallery safely with disabled navigation and an available close action", () => {
  render(<LocationGallery photos={[]} onClose={() => {}} />);
  expect(screen.getByText("0 photos")).toBeInTheDocument();
  expect(screen.getByText("Photos at this location are no longer available.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Previous photo" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Next photo" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Close gallery" })).toBeEnabled();
});
