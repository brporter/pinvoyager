import { StrictMode } from "react";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useUpdateVoyage } from "../../api/hooks";
import { voyage } from "../../screens/__tests__/support";
import { VoyageShareDialog } from "../VoyageShareDialog";

vi.mock("../../api/hooks", () => ({ useUpdateVoyage: vi.fn() }));
const mutate = vi.fn();
function pointer(node: HTMLElement, type: string, options: MouseEventInit) {
  fireEvent(node, new MouseEvent(type, { bubbles: true, ...options }));
}
beforeEach(() => {
  mutate.mockReset();
  vi.mocked(useUpdateVoyage).mockReturnValue({
    mutate,
    error: null,
    isPending: false,
  } as unknown as ReturnType<typeof useUpdateVoyage>);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it("shows the canonical public URL, copies it, confirms success, and closes", async () => {
  const user = userEvent.setup();
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  const onClose = vi.fn();
  render(
    <VoyageShareDialog voyage={voyage({ published: true })} canPublish={false} onClose={onClose} />,
  );
  const dialog = screen.getByRole("dialog", { name: "Share voyage" });
  const url = `${window.location.origin}/voyages/${voyage().id}`;
  expect(within(dialog).getByRole("textbox", { name: "Voyage URL" })).toHaveValue(url);
  expect(within(dialog).queryByRole("button", { name: "Publish" })).not.toBeInTheDocument();
  await user.click(within(dialog).getByRole("button", { name: "Copy URL" }));
  expect(writeText).toHaveBeenCalledWith(url);
  expect(within(dialog).getByRole("status")).toHaveTextContent("URL copied");
  await user.click(within(dialog).getByRole("button", { name: "Close share dialog" }));
  expect(onClose).toHaveBeenCalledOnce();
});

it("reveals a draft's URL only after publishing succeeds", async () => {
  const user = userEvent.setup();
  let finish!: () => void;
  mutate.mockImplementation((_patch, options) => {
    finish = () => options.onSuccess();
  });
  render(<VoyageShareDialog voyage={voyage({ published: false })} canPublish onClose={vi.fn()} />);
  expect(
    screen.getByText(
      "Publish this voyage to enable sharing. Published voyages are visible to everyone.",
    ),
  ).toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Voyage URL" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Publish" }));
  expect(mutate).toHaveBeenCalledWith({ published: true }, expect.any(Object));
  expect(screen.queryByRole("button", { name: "Copy URL" })).not.toBeInTheDocument();
  act(finish);
  expect(screen.getByRole("textbox", { name: "Voyage URL" })).toHaveValue(
    `${window.location.origin}/voyages/${voyage().id}`,
  );
  expect(screen.getByRole("button", { name: "Copy URL" })).toBeInTheDocument();
});

it("retains the draft prompt on publication failure and disables Publish while pending", () => {
  vi.mocked(useUpdateVoyage).mockReturnValue({
    mutate,
    error: new Error("A voyage name is required"),
    isPending: true,
  } as unknown as ReturnType<typeof useUpdateVoyage>);
  render(<VoyageShareDialog voyage={voyage({ published: false })} canPublish onClose={vi.fn()} />);
  expect(screen.getByRole("alert")).toHaveTextContent("A voyage name is required");
  expect(screen.getByRole("button", { name: "Publish" })).toBeDisabled();
  expect(screen.queryByRole("textbox", { name: "Voyage URL" })).not.toBeInTheDocument();
});

it("does not offer publication to a non-owner", () => {
  render(
    <VoyageShareDialog
      voyage={voyage({ published: false })}
      canPublish={false}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByText("Only the owner can publish this voyage.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Publish" })).not.toBeInTheDocument();
});

it("keeps the URL available for manual copy when clipboard access fails", async () => {
  const user = userEvent.setup();
  vi.stubGlobal("navigator", {
    clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
  });
  render(<VoyageShareDialog voyage={voyage()} canPublish onClose={vi.fn()} />);
  await user.click(screen.getByRole("button", { name: "Copy URL" }));
  expect(screen.getByRole("alert")).toHaveTextContent("Copy the URL manually");
  expect(screen.queryByText("URL copied")).not.toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Voyage URL" })).toHaveAttribute("readonly");
});

it("waits for clipboard success before showing confirmation and allows retry after a failure", async () => {
  const user = userEvent.setup();
  let finish!: () => void;
  const writeText = vi
    .fn()
    .mockRejectedValueOnce(new Error("denied"))
    .mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  render(<VoyageShareDialog voyage={voyage()} canPublish onClose={vi.fn()} />);
  await user.click(screen.getByRole("button", { name: "Copy URL" }));
  expect(screen.getByRole("alert")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Copy URL" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy URL" })).toBeDisabled();
  expect(screen.queryByText("URL copied")).not.toBeInTheDocument();
  await act(async () => {
    finish();
  });
  expect(screen.getByRole("status")).toHaveTextContent("URL copied");
  expect(screen.getByRole("button", { name: "Copy URL" })).toBeEnabled();
});

it("dismisses outside clicks without dismissing inside clicks, drags, or right clicks", () => {
  const onClose = vi.fn();
  render(<VoyageShareDialog voyage={voyage()} canPublish onClose={onClose} />);
  const dialog = screen.getByRole("dialog") as HTMLDialogElement;
  vi.spyOn(dialog, "getBoundingClientRect").mockReturnValue({
    left: 10,
    top: 10,
    right: 100,
    bottom: 100,
  } as DOMRect);
  pointer(dialog, "pointerdown", { button: 0, clientX: 50, clientY: 50 });
  pointer(dialog, "click", { button: 0, clientX: 200, clientY: 50 });
  pointer(screen.getByRole("textbox"), "pointerdown", { button: 0, clientX: 0, clientY: 0 });
  pointer(dialog, "click", { button: 0, clientX: 0, clientY: 0 });
  pointer(dialog, "pointerdown", { button: 2, clientX: 0, clientY: 0 });
  pointer(dialog, "click", { button: 2, clientX: 0, clientY: 0 });
  pointer(dialog, "pointerdown", { button: 0, clientX: 0, clientY: 0 });
  pointer(dialog, "click", { button: 2, clientX: 0, clientY: 0 });
  pointer(dialog, "pointerdown", { button: 0, clientX: 0, clientY: 0 });
  pointer(dialog, "click", { button: 0, clientX: 50, clientY: 50 });
  pointer(dialog, "pointerdown", { button: 0, clientX: 0, clientY: 0 });
  pointer(screen.getByRole("textbox"), "click", { button: 0, clientX: 50, clientY: 50 });
  expect(onClose).not.toHaveBeenCalled();
  for (const [clientX, clientY] of [
    [0, 50],
    [50, 0],
    [200, 50],
    [50, 200],
  ]) {
    act(() => dialog.showModal());
    pointer(dialog, "pointerdown", { button: 0, clientX, clientY });
    pointer(dialog, "click", { button: 0, clientX, clientY });
  }
  expect(onClose).toHaveBeenCalledTimes(4);
});

it("ignores StrictMode cleanup and queued close events while reopened", () => {
  const onClose = vi.fn();
  const view = render(
    <StrictMode>
      <VoyageShareDialog voyage={voyage()} canPublish onClose={onClose} />
    </StrictMode>,
  );
  const dialog = screen.getByRole("dialog") as HTMLDialogElement;
  fireEvent(dialog, new Event("close"));
  expect(onClose).not.toHaveBeenCalled();
  act(() => dialog.close());
  expect(onClose).toHaveBeenCalledOnce();
  view.unmount();
  expect(onClose).toHaveBeenCalledOnce();
});
