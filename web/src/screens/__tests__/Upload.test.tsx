/**
 * Tests for the two pure functions behind the upload screen.
 *
 * These are where the screen's stated purpose lives. The design's point is that
 * a real batch is mixed — some files upload, some have no location, some the
 * server rejects for different reasons — and the interface has to make each
 * outcome visible on its own row. `reasonFor` decides what a failed row says,
 * and `tally` decides what the summary counts. Both are exported precisely so
 * they can be checked without driving the whole screen.
 */

import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiRequestError, NetworkError, attachPhotos, createVoyage } from "../../api/client";
import type { Photo } from "../../api/types";
import { reasonFor, tally } from "../Upload";
import { Upload } from "../Upload";
import { ADA, authState, renderScreen, voyage } from "./support";

const screenMocks = vi.hoisted(() => ({
  attachPhotos: vi.fn(),
  createVoyage: vi.fn(),
  upload: vi.fn(),
  voyages: vi.fn(),
}));

vi.mock("../../api/hooks", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/hooks")>();
  return {
    ...actual,
    useUploadPhotos: () => ({ mutateAsync: screenMocks.upload }),
    useVoyages: screenMocks.voyages,
  };
});

vi.mock("../../api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/client")>();
  return {
    ...actual,
    attachPhotos: screenMocks.attachPhotos,
    createVoyage: screenMocks.createVoyage,
  };
});

beforeEach(() => {
  screenMocks.upload.mockReset().mockResolvedValue({ photos: [photo()] });
  screenMocks.attachPhotos.mockReset().mockResolvedValue({});
  screenMocks.createVoyage.mockReset().mockResolvedValue({ id: "new-voyage" });
  screenMocks.voyages.mockReset().mockReturnValue({
    data: { pages: [{ voyages: [voyage()] }] },
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** A file of a stated size. jsdom computes `size` from the real contents, so a
 *  "26 MB" file would otherwise mean allocating 26 MB in a test. */
function fileOfSize(bytes: number, name = "photo.jpg"): File {
  const file = new File([new Uint8Array(8)], name, { type: "image/jpeg" });
  Object.defineProperty(file, "size", { value: bytes });
  return file;
}

function apiError(status: number, code: string, message = "server prose"): ApiRequestError {
  return new ApiRequestError({ status, code, message, requestId: "req-1" });
}

function photo(overrides: Partial<Photo> = {}): Photo {
  return {
    id: "01a00000-0000-7000-8000-000000000001",
    width: 4032,
    height: 3024,
    byte_size: 4_200_000,
    original_format: "jpeg",
    captured_at: "2026-02-14T09:12:00Z",
    latitude: 43.06,
    longitude: 141.35,
    placed: true,
    variant_status: "ready",
    created_at: "2026-08-16T09:00:00Z",
    ...overrides,
  };
}

function CurrentPath() {
  return <output data-testid="current-path">{useLocation().pathname}</output>;
}

describe("reasonFor", () => {
  /** The four server rejections must read differently. A batch where an
   *  oversized file and a PDF both say "upload failed" tells the user nothing
   *  about which one to fix or how. */
  it("gives each server rejection its own distinct sentence", () => {
    const file = fileOfSize(4_200_000);

    const messages = [
      reasonFor(apiError(413, "too_large"), fileOfSize(26_000_000)).message,
      reasonFor(apiError(415, "not_an_image"), file).message,
      reasonFor(apiError(422, "unprocessable_image"), file).message,
      reasonFor(apiError(429, "rate_limited"), file).message,
    ];

    expect(new Set(messages).size).toBe(messages.length);
  });

  /** A limit quoted without the file's own size leaves the user guessing how
   *  far over they are. */
  it("names the file's actual size when it is too large", () => {
    const reason = reasonFor(apiError(413, "too_large"), fileOfSize(26_000_000));

    expect(reason.message).toContain("26 MB");
    expect(reason.message).toContain("25 MB");
  });

  /** Retrying a file the server will always refuse just wastes the user's
   *  time; retrying one that failed in transit is exactly right. The flag is
   *  what the row uses to decide whether to offer a Retry button. */
  it("marks permanent rejections as not retryable and transient ones as retryable", () => {
    const file = fileOfSize(4_200_000);

    expect(reasonFor(apiError(413, "too_large"), fileOfSize(26_000_000)).retryable).toBe(false);
    expect(reasonFor(apiError(415, "not_an_image"), file).retryable).toBe(false);
    expect(reasonFor(apiError(422, "unprocessable_image"), file).retryable).toBe(false);

    expect(reasonFor(apiError(429, "rate_limited"), file).retryable).toBe(true);
    expect(reasonFor(new NetworkError("offline", null), file).retryable).toBe(true);
  });

  /** An expired session is not a problem with the file, and retrying it would
   *  fail identically. The user has to sign in again. */
  it("tells the user to sign in again when the session ended", () => {
    const reason = reasonFor(apiError(401, "unauthorized"), fileOfSize(4_200_000));

    expect(reason.message.toLowerCase()).toContain("sign in");
    expect(reason.retryable).toBe(false);
  });

  it("handles delayed rate limits and unknown client and server errors", () => {
    const file = fileOfSize(4_200_000);
    const delayed = new ApiRequestError({
      status: 429,
      code: "rate_limited",
      message: "Wait",
      requestId: null,
      retryAfterSeconds: 0.2,
    });
    expect(reasonFor(delayed, file).message).toContain("in 1 seconds");

    expect(reasonFor(apiError(400, "unknown", "Bad request"), file)).toEqual({
      message: "Bad request",
      retryable: false,
    });
    const serverError = new ApiRequestError({
      status: 503,
      code: "unknown",
      message: "Server error",
      requestId: null,
    });
    expect(reasonFor(serverError, file)).toEqual({
      message: "The server failed on this file",
      retryable: true,
    });
  });

  /** A proxy returning an HTML error page, or any shape that is not ours, must
   *  still produce a row rather than throwing and taking the batch with it. */
  it("produces a reason for an error shape it does not recognize", () => {
    const reason = reasonFor(new Error("something else entirely"), fileOfSize(1000));

    expect(reason.message).toBeTruthy();
    expect(reason.retryable).toBe(true);
  });

  it("includes a request reference for unexpected server errors", () => {
    const reason = reasonFor(apiError(503, "unexpected", "backend unavailable"), fileOfSize(1000));

    expect(reason.message).toBe("The server failed on this file — reference req-1");
    expect(reason.retryable).toBe(true);
  });
});

describe("tally", () => {
  const base = { id: "1", file: fileOfSize(1000), progress: 100 };

  /** The whole reason this is one function. A summary that counted only
   *  successes would let someone close the page believing a batch finished
   *  when two files never made it. */
  it("counts every outcome, including the failures", () => {
    const counts = tally([
      { ...base, id: "a", status: "uploaded", photo: photo() },
      { ...base, id: "b", status: "uploaded", photo: photo() },
      { ...base, id: "c", status: "error", failure: { message: "no", retryable: false } },
      { ...base, id: "d", status: "uploading", progress: 40 },
    ]);

    expect(counts).toEqual({ total: 4, uploaded: 2, unplaced: 0, failed: 1, pending: 1 });
  });

  /** A photo with no coordinates uploaded successfully. It is both — counted
   *  as uploaded and flagged as needing placement — because the design shows
   *  "4 of 6 uploaded" and "1 has no location" as separate facts about the
   *  same batch, not as alternatives. */
  it("counts a photo with no location as uploaded and as unplaced", () => {
    const counts = tally([
      { ...base, id: "a", status: "uploaded", photo: photo() },
      { ...base, id: "b", status: "uploaded", photo: photo({ latitude: null, longitude: null }) },
    ]);

    expect(counts.uploaded).toBe(2);
    expect(counts.unplaced).toBe(1);
  });

  it("reports nothing for an empty batch rather than failing", () => {
    expect(tally([])).toEqual({ total: 0, uploaded: 0, unplaced: 0, failed: 0, pending: 0 });
  });
});

describe("Upload screen", () => {
  it("uploads a chosen photo and advances from processing to review", async () => {
    const user = userEvent.setup();
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });

    expect(screen.getByRole("heading", { name: "Upload photos" })).toBeInTheDocument();
    const input = container.querySelector<HTMLInputElement>('input[type="file"]');
    expect(input).not.toBeNull();
    await user.upload(input!, new File(["image"], "hokkaido.jpg", { type: "image/jpeg" }));

    expect(await screen.findByRole("heading", { name: "Uploaded 1 of 1" })).toBeInTheDocument();
    expect(screen.getByText("1 of 1 photo uploaded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review 1 photo" })).toBeInTheDocument();
    expect(screen.getByText("hokkaido.jpg")).toBeInTheDocument();
    expect(screenMocks.upload).toHaveBeenCalledWith(
      expect.objectContaining({
        files: [expect.objectContaining({ name: "hokkaido.jpg" })],
        signal: expect.any(AbortSignal),
        onProgress: expect.any(Function),
      }),
    );

    await user.click(screen.getByRole("button", { name: "Review 1 photo" }));
    expect(screen.getByRole("heading", { name: "Review" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Where these photos landed — 1 on the map" }))
      .toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Add to voyage" })).toHaveValue(
      `voyage:${voyage().id}`,
    );
    expect(screen.getByRole("button", { name: "Add 1 photo" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByRole("heading", { name: "Uploaded 1 of 1" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Review 1 photo" }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Add to voyage" }),
      `voyage:${voyage().id}`,
    );
    expect(screen.getByRole("combobox", { name: "Add to voyage" })).toHaveValue(
      `voyage:${voyage().id}`,
    );
  });

  it("lets a failed batch be restarted without keeping its failed rows", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockRejectedValue(new NetworkError("offline", null));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "offline.jpg", { type: "image/jpeg" }));

    expect(await screen.findByText("Could not be sent — check your connection")).toBeInTheDocument();
    expect(screen.getByText("Nothing was uploaded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Choose different files" }));
    expect(screen.getByRole("heading", { name: "Upload photos" })).toBeInTheDocument();
    expect(screen.queryByText("offline.jpg")).not.toBeInTheDocument();
    expect(input).toHaveValue("");
  });

  it("shows byte progress, processing, and completion as the upload advances", async () => {
    const user = userEvent.setup();
    let reportProgress!: (fraction: number | null) => void;
    let resolveUpload!: (result: { photos: Photo[] }) => void;
    screenMocks.upload.mockImplementation(({ onProgress }) => {
      reportProgress = onProgress;
      return new Promise((resolve) => {
        resolveUpload = resolve;
      });
    });
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "progress.jpg", { type: "image/jpeg" }));

    const progress = await screen.findByRole("progressbar", {
      name: "Upload progress for progress.jpg",
    });
    act(() => reportProgress(0.456));
    expect(progress).toHaveAttribute("aria-valuenow", "46");

    act(() => reportProgress(1));
    expect(progress).toHaveAttribute("aria-valuenow", "100");
    expect(screen.getByText(/reading location data/)).toBeInTheDocument();

    act(() => reportProgress(null));
    expect(progress).toHaveAttribute("aria-valuenow", "100");
    await act(async () => resolveUpload({ photos: [photo()] }));
    await waitFor(() => expect(screen.getByText(/43\.06°, 141\.35°/)).toBeInTheDocument());
  });

  it("turns an empty response into a retryable row and succeeds on retry", async () => {
    const user = userEvent.setup();
    screenMocks.upload
      .mockResolvedValueOnce({ photos: [] })
      .mockResolvedValueOnce({ photos: [photo()] });
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "empty-response.jpg", { type: "image/jpeg" }));

    expect(
      await screen.findByText("The server accepted the file but returned no photo"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));

    expect(await screen.findByText(/43\.06°, 141\.35°/)).toBeInTheDocument();
    expect(screenMocks.upload).toHaveBeenCalledTimes(2);
  });

  it("cancels only the in-flight file and shows the retry option", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockImplementation(({ signal }) => new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
    }));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "cancel-me.jpg", { type: "image/jpeg" }));
    const request = screenMocks.upload.mock.calls[0][0];

    await user.click(screen.getByRole("button", { name: "Cancel upload of cancel-me.jpg" }));

    expect(request.signal.aborted).toBe(true);
    expect(await screen.findByText("Canceled")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("creates a new voyage and attaches only placed photos when unplaced photos are excluded", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockImplementation(({ files }) => Promise.resolve({
      photos: [
        files[0].name === "placed.jpg"
          ? photo()
          : photo({ id: "unplaced-id", latitude: null, longitude: null, placed: false }),
      ],
    }));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, [
      new File(["image"], "placed.jpg", { type: "image/jpeg" }),
      new File(["image"], "unplaced.jpg", { type: "image/jpeg" }),
    ]);

    expect(await screen.findByRole("button", { name: "Review 2 photos" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Review 2 photos" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Add to voyage" }), "new");
    await user.type(screen.getByRole("textbox", { name: "Voyage title" }), "A new trip");
    await user.click(screen.getByRole("checkbox", { name: "Include it anyway" }));
    expect(screen.getByRole("checkbox", { name: "Include it anyway" })).not.toBeChecked();

    await user.click(screen.getByRole("button", { name: "Add 1 photo" }));

    await waitFor(() => expect(createVoyage).toHaveBeenCalledWith({ title: "A new trip" }));
    expect(attachPhotos).toHaveBeenCalledWith("new-voyage", [photo().id]);
  });

  it("creates an untitled voyage and spreads distinct locations across the review map", async () => {
    const user = userEvent.setup();
    const secondPhoto = photo({
      id: "second-photo",
      latitude: 34.7,
      longitude: 135.5,
      placed: true,
    });
    screenMocks.upload.mockImplementation(({ files }) => Promise.resolve({
      photos: [files[0].name === "second.jpg" ? secondPhoto : photo()],
    }));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(container.querySelector<HTMLInputElement>('input[type="file"]')!, [
      new File(["a"], "first.jpg", { type: "image/jpeg" }),
      new File(["b"], "second.jpg", { type: "image/jpeg" }),
    ]);
    await user.click(await screen.findByRole("button", { name: "Review 2 photos" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Add to voyage" }), "new");

    expect(screen.getByRole("group", { name: "Where these photos landed — 2 on the map" })
      .querySelectorAll(".pv-pin")).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "Add 2 photos" }));

    await waitFor(() => expect(createVoyage).toHaveBeenCalledWith({}));
    expect(attachPhotos).toHaveBeenCalledWith("new-voyage", [photo().id, secondPhoto.id]);
  });

  it("removes a permanent failure from the batch", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockRejectedValue(apiError(413, "too_large"));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, fileOfSize(26_000_000, "oversized.jpg"));

    expect(await screen.findByText(/Too large/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove oversized.jpg" }));

    expect(screen.queryByText("oversized.jpg")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Uploaded 0 of 0" })).toBeInTheDocument();
  });

  it("retries loading voyages from the review step", async () => {
    const user = userEvent.setup();
    const refetch = vi.fn();
    const failedVoyages = {
      data: undefined,
      isPending: false,
      isError: true,
      refetch,
    };
    screenMocks.voyages.mockReturnValue(failedVoyages);
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "retry-voyages.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(refetch).toHaveBeenCalledOnce();
  });

  it("shows a loading placeholder while the review destination list is pending", async () => {
    const user = userEvent.setup();
    screenMocks.voyages.mockReturnValue({
      data: undefined,
      isPending: true,
      isError: false,
      refetch: vi.fn(),
    });
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "pending-list.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));

    expect(screen.getByText("Loading your voyages…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Add 1 photo/ })).not.toBeInTheDocument();
  });

  it("keeps the empty destination list useful and identifies draft options", async () => {
    const user = userEvent.setup();
    screenMocks.voyages.mockReturnValue({
      data: { pages: [{ voyages: [] }] },
      isPending: false,
      isError: false,
      refetch: vi.fn(),
    });
    const { container, unmount } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "empty-list.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));

    expect(screen.getByText(/You have no voyages yet/)).toBeInTheDocument();
    unmount();

    screenMocks.voyages.mockReturnValue({
      data: { pages: [{ voyages: [voyage({ published: false })] }] },
      isPending: false,
      isError: false,
      refetch: vi.fn(),
    });
    const draftView = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      draftView.container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "draft-option.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));

    expect(screen.getByRole("option", { name: /draft/i })).toBeInTheDocument();
  });

  it("keeps photos in the library without creating or attaching a voyage", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockResolvedValue({
      photos: [photo({ latitude: null, longitude: null, placed: false })],
    });
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "library.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Add to voyage" }),
      "library",
    );

    await user.click(screen.getByRole("button", { name: "Keep 1 photo in my library" }));

    await waitFor(() => expect(screenMocks.upload).toHaveBeenCalledOnce());
    expect(createVoyage).not.toHaveBeenCalled();
    expect(attachPhotos).not.toHaveBeenCalled();
  });

  it("returns an anonymous visitor to the home page after keeping photos in the library", async () => {
    const user = userEvent.setup();
    screenMocks.voyages.mockReturnValue({
      data: { pages: [{ voyages: [] }] },
      isPending: false,
      isError: false,
      refetch: vi.fn(),
    });
    const { container } = renderScreen(<><Upload /><CurrentPath /></>, {
      auth: authState(null),
    });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "anonymous-library.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));
    await user.click(screen.getByRole("button", { name: "Keep 1 photo in my library" }));

    await waitFor(() => expect(screen.getByTestId("current-path")).toHaveTextContent("/"));
  });

  it("shows an error when adding photos to the selected voyage fails", async () => {
    const user = userEvent.setup();
    vi.mocked(attachPhotos).mockRejectedValueOnce(new Error("database unavailable"));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "attach-error.jpg", { type: "image/jpeg" }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));
    await user.click(screen.getByRole("button", { name: "Add 1 photo" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("database unavailable");
  });

  it("aborts the pending upload and revokes its preview URL on unmount", async () => {
    const user = userEvent.setup();
    const createUrl = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:test-preview");
    const revokeUrl = vi.spyOn(URL, "revokeObjectURL");
    screenMocks.upload.mockImplementation(({ signal }) => new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
    }));
    const { container, unmount } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "cleanup.jpg", { type: "image/jpeg" }),
    );
    const { signal } = screenMocks.upload.mock.calls[0][0];

    unmount();

    expect(signal.aborted).toBe(true);
    expect(createUrl).toHaveBeenCalledOnce();
    expect(revokeUrl).toHaveBeenCalledWith("blob:test-preview");
  });

  it("continues without a thumbnail when the object URL API throws", async () => {
    const user = userEvent.setup();
    vi.spyOn(URL, "createObjectURL").mockImplementation(() => {
      throw new Error("object URLs unavailable");
    });
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "no-preview.jpg", { type: "image/jpeg" }),
    );

    expect(await screen.findByText("no-preview.jpg")).toBeInTheDocument();
    expect(container.querySelector(".pv-upload-thumb img")).toBeNull();
  });

  it("continues without a thumbnail when the object URL API is absent", async () => {
    const user = userEvent.setup();
    const descriptor = Object.getOwnPropertyDescriptor(URL, "createObjectURL");
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: undefined });
    try {
      const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
      await user.upload(
        container.querySelector<HTMLInputElement>('input[type="file"]')!,
        new File(["image"], "missing-preview.jpg", { type: "image/jpeg" }),
      );

      expect(await screen.findByText("missing-preview.jpg")).toBeInTheDocument();
      expect(container.querySelector(".pv-upload-thumb img")).toBeNull();
    } finally {
      if (descriptor === undefined) delete (URL as { createObjectURL?: typeof URL.createObjectURL }).createObjectURL;
      else Object.defineProperty(URL, "createObjectURL", descriptor);
    }
  });

  it("skips preview cleanup when the revoke API is absent", async () => {
    const user = userEvent.setup();
    const descriptor = Object.getOwnPropertyDescriptor(URL, "revokeObjectURL");
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: undefined });
    try {
      const { container, unmount } = renderScreen(<Upload />, { auth: authState(ADA) });
      await user.upload(
        container.querySelector<HTMLInputElement>('input[type="file"]')!,
        new File(["image"], "no-revoke.jpg", { type: "image/jpeg" }),
      );
      expect(await screen.findByText("no-revoke.jpg")).toBeInTheDocument();
      unmount();
    } finally {
      if (descriptor === undefined) delete (URL as { revokeObjectURL?: typeof URL.revokeObjectURL }).revokeObjectURL;
      else Object.defineProperty(URL, "revokeObjectURL", descriptor);
    }
  });
});
