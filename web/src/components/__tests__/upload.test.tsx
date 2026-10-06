import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Dropzone } from "../Dropzone";
import { UploadRow } from "../UploadRow";

function file(name: string, type: string) {
  return new File(["x"], name, { type });
}

/** jsdom has no DataTransfer, so drags are described by hand. `items` is what
 *  the browser exposes before the drop and `files` is what arrives with it. */
function transfer(files: File[]): DataTransfer {
  return {
    files,
    items: files.map((f) => ({ kind: "file", type: f.type })),
    types: ["Files"],
  } as unknown as DataTransfer;
}

function zone(): HTMLElement {
  const element = document.querySelector<HTMLElement>(".pv-dropzone");
  if (element == null) throw new Error("dropzone not rendered");
  return element;
}

describe("Dropzone", () => {
  it("warns about an unsupported file while it is still being held", () => {
    render(<Dropzone onFiles={() => {}} />);

    fireEvent.dragEnter(zone(), {
      dataTransfer: transfer([
        file("a.jpg", "image/jpeg"),
        file("notes.pdf", "application/pdf"),
        file("b.png", "image/png"),
      ]),
    });

    expect(screen.getByText("Only image files are supported")).toBeInTheDocument();
    expect(screen.getByText("1 of these 3 files are not images")).toBeInTheDocument();
  });

  it("counts what an acceptable drag is carrying", () => {
    render(<Dropzone onFiles={() => {}} />);

    fireEvent.dragEnter(zone(), {
      dataTransfer: transfer([file("a.jpg", "image/jpeg"), file("b.jpg", "image/jpeg")]),
    });

    expect(screen.getByText("Release to upload 2 photos")).toBeInTheDocument();
  });

  it("returns to idle when the drag leaves", () => {
    render(<Dropzone onFiles={() => {}} />);
    const element = zone();

    fireEvent.dragEnter(element, { dataTransfer: transfer([file("a.jpg", "image/jpeg")]) });
    fireEvent.dragLeave(element, { dataTransfer: transfer([file("a.jpg", "image/jpeg")]) });

    expect(screen.getByText("Drop photos here")).toBeInTheDocument();
  });

  it("tracks nested drag enters until the pointer leaves the whole zone", () => {
    render(<Dropzone onFiles={() => {}} />);
    const element = zone();
    const files = transfer([file("a.jpg", "image/jpeg")]);

    fireEvent.dragEnter(element, { dataTransfer: files });
    fireEvent.dragEnter(element, { dataTransfer: files });
    fireEvent.dragLeave(element, { dataTransfer: files });
    expect(element).toHaveAttribute("data-state", "dragover");
    fireEvent.dragLeave(element, { dataTransfer: files });
    expect(element).not.toHaveAttribute("data-state");
  });

  it("accepts a dragover with no inspectable item list", () => {
    render(<Dropzone onFiles={() => {}} />);
    const element = zone();

    fireEvent.dragOver(element);

    expect(screen.getByText("Release to upload 0 photos")).toBeInTheDocument();
    expect(element).toHaveAttribute("data-state", "dragover");
  });

  it("keeps the current drag count when dragover follows a nested enter", () => {
    render(<Dropzone onFiles={() => {}} />);
    const element = zone();
    const files = transfer([file("a.jpg", "image/jpeg")]);

    fireEvent.dragEnter(element, { dataTransfer: files });
    fireEvent.dragOver(element, { dataTransfer: files });

    expect(element).toHaveAttribute("data-state", "dragover");
    expect(screen.getByText("Release to upload 1 photo")).toBeInTheDocument();
  });

  it("treats unknown file types as possible images and ignores non-file drag items", () => {
    render(<Dropzone onFiles={() => {}} />);
    const element = zone();
    const dataTransfer = {
      items: [
        { kind: "string", type: "text/plain" },
        { kind: "file", type: "" },
      ],
    } as unknown as DataTransfer;

    fireEvent.dragEnter(element, { dataTransfer });

    expect(screen.getByText("Release to upload 1 photo")).toBeInTheDocument();
  });

  it("hands over every dropped file, including the ones it warned about", () => {
    // A batch that quietly loses two files is worse than one that reports them:
    // the per-file list is where "Not an image file — skipped" is shown.
    const onFiles = vi.fn();
    render(<Dropzone onFiles={onFiles} />);

    const files = [file("a.jpg", "image/jpeg"), file("notes.pdf", "application/pdf")];
    fireEvent.drop(zone(), { dataTransfer: transfer(files) });

    expect(onFiles).toHaveBeenCalledOnce();
    expect(onFiles.mock.calls[0]?.[0]).toHaveLength(2);
    expect(screen.getByText("Drop photos here")).toBeInTheDocument();
  });

  it("ignores drops and file selections that contain no files", () => {
    const onFiles = vi.fn();
    render(<Dropzone onFiles={onFiles} />);
    const input = document.querySelector<HTMLInputElement>('input[type="file"]');
    if (input == null) throw new Error("no file input");

    fireEvent.drop(zone());
    fireEvent.drop(zone(), { dataTransfer: transfer([]) });
    fireEvent.change(input, { target: { files: null } });

    expect(onFiles).not.toHaveBeenCalled();
    expect(input.value).toBe("");
  });

  it("offers a real control for choosing files rather than a pointer-only one", async () => {
    render(<Dropzone onFiles={() => {}} />);

    const input = document.querySelector<HTMLInputElement>('input[type="file"]');
    if (input == null) throw new Error("no file input");
    const click = vi.spyOn(input, "click").mockImplementation(() => {});

    await userEvent.tab();
    const button = screen.getByRole("button", { name: "choose files" });
    expect(button).toHaveFocus();

    await userEvent.keyboard("{Enter}");
    expect(click).toHaveBeenCalled();
  });

  it("does not react to a drag when disabled", () => {
    const onFiles = vi.fn();
    render(<Dropzone onFiles={onFiles} disabled />);
    const element = zone();

    fireEvent.dragEnter(element, { dataTransfer: transfer([file("a.jpg", "image/jpeg")]) });
    fireEvent.dragOver(element, { dataTransfer: transfer([file("a.jpg", "image/jpeg")]) });
    fireEvent.dragLeave(element, { dataTransfer: transfer([file("a.jpg", "image/jpeg")]) });
    fireEvent.drop(element, { dataTransfer: transfer([file("a.jpg", "image/jpeg")]) });

    expect(screen.getByText("Drop photos here")).toBeInTheDocument();
    expect(onFiles).not.toHaveBeenCalled();
  });
});

describe("UploadRow", () => {
  it("exposes upload progress rather than only drawing it", () => {
    render(
      <UploadRow name="IMG_4821.jpg" status="uploading" progress={68} detail="4.2 MB · uploading" />,
    );

    const bar = screen.getByRole("progressbar", { name: "Upload progress for IMG_4821.jpg" });
    expect(bar).toHaveAttribute("aria-valuenow", "68");
  });

  it("names its cancel button after the file it cancels", async () => {
    // "Cancel" repeated down a list of six rows tells a screen-reader user
    // nothing about which upload they are about to stop.
    const onCancel = vi.fn();
    render(
      <UploadRow name="IMG_4821.jpg" status="uploading" progress={30} onCancel={onCancel} />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Cancel upload of IMG_4821.jpg" }),
    );

    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("shows why a file failed and offers a retry", async () => {
    const onRetry = vi.fn();
    render(
      <UploadRow
        name="DSC_0099.tif"
        status="error"
        error="Too large — 68 MB, limit is 25 MB"
        onRetry={onRetry}
      />,
    );

    expect(screen.getByText("Too large — 68 MB, limit is 25 MB")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("distinguishes an uploaded photo that still needs placing", () => {
    render(
      <UploadRow
        name="Screenshot 2026-02-14.png"
        status="uploaded"
        unplaced
        detail="1.1 MB · uploaded, needs placing"
      />,
    );

    expect(screen.getByText("No location")).toBeInTheDocument();
    expect(screen.getByText("Uploaded")).toBeInTheDocument();
  });

  it("has no progress bar once the file has landed", () => {
    render(<UploadRow name="IMG_4823.jpg" status="uploaded" detail="2.1 MB · 43.06°, 141.35°" />);

    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });
});
