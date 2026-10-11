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

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiRequestError, NetworkError, createVoyage } from "../../api/client";
import type { Photo } from "../../api/types";
import { reasonFor, tally } from "../upload/model";
import { Upload } from "../Upload";
import { ADA, authState, renderScreen, voyage } from "./support";
import { __maps, __resetMaps } from "../../../__mocks__/maplibre-gl";

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
  __resetMaps();
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

async function enterDetails(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Test voyage");
  await user.type(
    screen.getByRole("textbox", { name: /Voyage Description/ }),
    "A trip with photos.",
  );
  await user.click(screen.getByRole("button", { name: "~1 km" }));
  await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
  await screen.findByRole("button", { name: "choose files" });
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
  it("requires an explicit precision choice even when the other voyage details are filled", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem("pinvoyager.default_granularity", "exact");
    renderScreen(<Upload />, { auth: authState(ADA) });
    window.localStorage.removeItem("pinvoyager.default_granularity");
    for (const name of ["Exact", "~1 km", "~10 km", "~50 km"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-pressed", "false");
    }
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Private places");
    await user.type(screen.getByRole("textbox", { name: /Voyage Description/ }), "A trip");
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Choose a location precision.");
    expect(createVoyage).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "~10 km" }));
    expect(screen.queryByText("Choose a location precision.")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    await waitFor(() =>
      expect(createVoyage).toHaveBeenCalledWith(
        { title: "Private places", description: "A trip", granularity: "km10" },
        { traceContext: {} },
      ),
    );
  });

  it("still requires a name and description after precision is selected", async () => {
    const user = userEvent.setup();
    renderScreen(<Upload />, { auth: authState(ADA) });
    await user.click(screen.getByRole("button", { name: "Exact" }));
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(screen.getByText("Enter a voyage name.")).toBeInTheDocument();
    expect(screen.getByText("Enter a voyage description.")).toBeInTheDocument();
    expect(screen.queryByText("Choose a location precision.")).not.toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Trip");
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(createVoyage).not.toHaveBeenCalled();
    expect(screen.getByText("Enter a voyage description.")).toBeInTheDocument();
  });

  it.each([
    ["Exact", "exact"],
    ["~1 km", "km1"],
    ["~10 km", "km10"],
    ["~50 km", "km50"],
  ])("creates a voyage using the selected precision %s", async (label, granularity) => {
    const user = userEvent.setup();
    renderScreen(<Upload />, { auth: authState(ADA) });
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Trip");
    await user.type(screen.getByRole("textbox", { name: /Voyage Description/ }), "Description");
    await user.click(screen.getByRole("button", { name: label }));
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    await waitFor(() =>
      expect(createVoyage).toHaveBeenCalledWith(
        { title: "Trip", description: "Description", granularity },
        { traceContext: {} },
      ),
    );
  });

  it("uploads directly into an existing voyage without creating another draft", async () => {
    const user = userEvent.setup();
    const { container } = renderScreen(<Upload />, {
      auth: authState(ADA),
      path: "/upload?voyage=v1",
    });
    expect(screen.queryByRole("textbox", { name: /Voyage Name/ })).not.toBeInTheDocument();
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "added.jpg", { type: "image/jpeg" }),
    );
    await waitFor(() =>
      expect(screenMocks.upload).toHaveBeenCalledWith(expect.objectContaining({ voyageId: "v1" })),
    );
    expect(createVoyage).not.toHaveBeenCalled();
  });

  it("shows queued files and reviews a partial batch containing only unplaced photos", async () => {
    const user = userEvent.setup();
    let finish!: (result: { photos: Photo[] }) => void;
    screenMocks.upload
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      )
      .mockRejectedValueOnce(new NetworkError("offline", null));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    await user.upload(container.querySelector<HTMLInputElement>('input[type="file"]')!, [
      new File(["first"], "first.jpg", { type: "image/jpeg" }),
      new File(["second"], "second.jpg", { type: "image/jpeg" }),
    ]);
    expect(await screen.findByText(/waiting/)).toBeInTheDocument();
    await act(async () =>
      finish({ photos: [photo({ latitude: null, longitude: null, placed: false })] }),
    );
    await user.click(await screen.findByRole("button", { name: "Review 1 photo" }));
    expect(screen.getByText("No location")).toBeInTheDocument();
    const map = await screen.findByRole("region", {
      name: "Where these photos landed — 0 on the map",
      busy: false,
    });
    expect(map.querySelectorAll(".pv-pin")).toHaveLength(0);
    expect(__maps.at(-1)!.options.center).toEqual([0, 20]);
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("requires details first and explains that photo uploads come next", async () => {
    const user = userEvent.setup();
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    expect(screen.getByRole("textbox", { name: /Voyage Name/ })).toBeRequired();
    expect(screen.getByRole("textbox", { name: /Voyage Description/ })).toBeRequired();
    expect(
      screen.getByText("You will be able to upload your photos at the next step."),
    ).toBeInTheDocument();
    expect(container.querySelector('input[type="file"]')).toBeNull();
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(screen.getByText("Enter a voyage name.")).toBeInTheDocument();
    expect(screen.getByText("Enter a voyage description.")).toBeInTheDocument();
    expect(createVoyage).not.toHaveBeenCalled();
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "   ");
    await user.type(screen.getByRole("textbox", { name: /Voyage Description/ }), "   ");
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(createVoyage).not.toHaveBeenCalled();
  });

  it("creates the draft with trimmed details before allowing file selection", async () => {
    const user = userEvent.setup();
    let finishCreate!: (value: { id: string }) => void;
    screenMocks.createVoyage.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishCreate = resolve;
        }),
    );
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "  Hokkaido  ");
    await user.type(screen.getByRole("textbox", { name: /Voyage Description/ }), "  Winter trip  ");
    await user.click(screen.getByRole("button", { name: "~10 km" }));
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    await waitFor(() =>
      expect(createVoyage).toHaveBeenCalledWith(
        { title: "Hokkaido", description: "Winter trip", granularity: "km10" },
        { traceContext: {} },
      ),
    );
    expect(container.querySelector('input[type="file"]')).toBeNull();
    expect(screen.getByRole("button", { name: "Next: upload photos" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "~10 km" })).toBeDisabled();
    expect(screenMocks.upload).not.toHaveBeenCalled();
    await act(async () => finishCreate({ id: "new-voyage" }));
    await screen.findByRole("button", { name: "choose files" });
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      new File(["image"], "trip.jpg", { type: "image/jpeg" }),
    );
    expect(await screen.findByRole("heading", { name: "Uploaded 1 of 1" })).toBeInTheDocument();
  });

  it("creates only one draft when Next is double clicked while creation is pending", async () => {
    const user = userEvent.setup();
    let finish!: (value: { id: string }) => void;
    screenMocks.createVoyage.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    renderScreen(<Upload />, { auth: authState(ADA) });
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Trip");
    await user.type(
      screen.getByRole("textbox", { name: /Voyage Description/ }),
      "Trip description",
    );
    await user.click(screen.getByRole("button", { name: "~1 km" }));
    await user.dblClick(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(createVoyage).toHaveBeenCalledTimes(1);
    await act(async () => finish({ id: "new-voyage" }));
    expect(await screen.findByRole("button", { name: "choose files" })).toBeInTheDocument();
  });

  it("submits voyage details with Enter and opens the file input with choose files", async () => {
    const user = userEvent.setup();
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.type(
      screen.getByRole("textbox", { name: /Voyage Description/ }),
      "Trip description",
    );
    await user.click(screen.getByRole("button", { name: "~1 km" }));
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Trip{Enter}");
    await screen.findByRole("button", { name: "choose files" });
    expect(createVoyage).toHaveBeenCalledTimes(1);
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    const click = vi.spyOn(input, "click");
    await user.click(screen.getByRole("button", { name: "choose files" }));
    expect(click).toHaveBeenCalledTimes(1);
    const files = [
      new File(["one"], "one.jpg", { type: "image/jpeg" }),
      new File(["two"], "two.jpg", { type: "image/jpeg" }),
    ];
    await user.upload(input, files);
    expect(await screen.findByRole("heading", { name: "Uploaded 2 of 2" })).toBeInTheDocument();
    expect(screenMocks.upload).toHaveBeenCalledTimes(2);
    expect(screenMocks.upload.mock.calls[1][0].idempotencyKey).not.toBe(
      screenMocks.upload.mock.calls[0][0].idempotencyKey,
    );
    expect(screenMocks.upload.mock.calls[0][0].idempotencyKey).toMatch(/^[0-9a-f-]{36}$/);
    expect(screenMocks.upload).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ files: [files[0]] }),
    );
    expect(screenMocks.upload).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ files: [files[1]] }),
    );
  });

  it("automatically uploads dropped photos at the second step", async () => {
    const user = userEvent.setup();
    const files = [new File(["image"], "dropped.heic", { type: "image/heic" })];
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    fireEvent.drop(container.querySelector(".pv-dropzone")!, { dataTransfer: { files } });
    expect(await screen.findByRole("heading", { name: "Uploaded 1 of 1" })).toBeInTheDocument();
    expect(screenMocks.upload).toHaveBeenCalledWith(
      expect.objectContaining({ files, voyageId: "new-voyage" }),
    );
  });

  it("uploads a chosen photo and advances from processing to review", async () => {
    const user = userEvent.setup();
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);

    expect(screen.getByRole("heading", { name: "Add photos" })).toBeInTheDocument();
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
    expect(
      await screen.findByRole("region", {
        name: "Where these photos landed — 1 on the map",
        busy: false,
      }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "View voyage" })).toBeInTheDocument();
    expect(__maps.at(-1)!.options.bounds).toEqual([
      [141.35, 43.06],
      [141.35, 43.06],
    ]);
    expect(__maps.at(-1)!.options.fitBoundsOptions).toMatchObject({ maxZoom: 16 });
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(__maps.at(-1)!.removed).toBe(true);
    expect(screen.getByRole("heading", { name: "Uploaded 1 of 1" })).toBeInTheDocument();
  });

  it("lets a failed batch be restarted without keeping its failed rows", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockRejectedValue(new NetworkError("offline", null));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "offline.jpg", { type: "image/jpeg" }));

    expect(
      await screen.findByText("Could not be sent — check your connection"),
    ).toBeInTheDocument();
    expect(screen.getByText("Nothing was uploaded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Choose different files" }));
    expect(screen.getByRole("heading", { name: "Add photos" })).toBeInTheDocument();
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
    await enterDetails(user);
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
    await enterDetails(user);
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "empty-response.jpg", { type: "image/jpeg" }));

    expect(
      await screen.findByText("The server accepted the file but returned no photo"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));

    expect(await screen.findByText(/43\.06°, 141\.35°/)).toBeInTheDocument();
    expect(screenMocks.upload).toHaveBeenCalledTimes(2);
    expect(screenMocks.upload.mock.calls[1][0].idempotencyKey).toBe(
      screenMocks.upload.mock.calls[0][0].idempotencyKey,
    );
    expect(screenMocks.upload.mock.calls[0][0].idempotencyKey).toMatch(/^[0-9a-f-]{36}$/);
  });

  it("cancels only the in-flight file and shows the retry option", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockImplementation(
      ({ signal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
        }),
    );
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, new File(["image"], "cancel-me.jpg", { type: "image/jpeg" }));
    const request = screenMocks.upload.mock.calls[0][0];

    await user.click(screen.getByRole("button", { name: "Cancel upload of cancel-me.jpg" }));

    expect(request.signal.aborted).toBe(true);
    expect(await screen.findByText("Canceled")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("keeps photos without GPS in the same voyage and reviews distinct locations", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockImplementation(({ files }) =>
      Promise.resolve({
        photos: [
          files[0].name === "unplaced.jpg"
            ? photo({ latitude: null, longitude: null, placed: false })
            : files[0].name === "missing-longitude.jpg"
              ? photo({ id: "partial", longitude: null })
              : files[0].name === "second.jpg"
                ? photo({ id: "p2", latitude: 34.7, longitude: 135.5 })
                : photo(),
        ],
      }),
    );
    const { container } = renderScreen(
      <>
        <Upload />
        <CurrentPath />
      </>,
      { auth: authState(ADA) },
    );
    await enterDetails(user);
    await user.upload(container.querySelector<HTMLInputElement>('input[type="file"]')!, [
      new File(["image"], "first.jpg", { type: "image/jpeg" }),
      new File(["image"], "second.jpg", { type: "image/jpeg" }),
      new File(["image"], "unplaced.jpg", { type: "image/jpeg" }),
      new File(["image"], "missing-longitude.jpg", { type: "image/jpeg" }),
    ]);
    await user.click(await screen.findByRole("button", { name: "Review 4 photos" }));
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.queryByText(/library/i)).not.toBeInTheDocument();
    const map = await screen.findByRole("region", {
      name: "Where these photos landed — 2 on the map",
      busy: false,
    });
    expect(map.querySelectorAll(".pv-pin")).toHaveLength(2);
    expect(
      Array.from(map.querySelectorAll<HTMLElement>("[data-lng]"), (marker) => [
        marker.dataset.lng,
        marker.dataset.lat,
      ]),
    ).toEqual([
      ["141.35", "43.06"],
      ["135.5", "34.7"],
    ]);
    expect(screen.getByText(/2 photos are not on the map/)).toBeInTheDocument();
    expect(__maps.filter((map) => !map.removed)).toHaveLength(1);
    expect(__maps.at(-1)!.options.bounds).toEqual([
      [135.5, 34.7],
      [141.35, 43.06],
    ]);
    expect(__maps.at(-1)!.options.style).toMatchObject({
      sources: {
        basemap: {
          type: "raster",
          tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
        },
      },
    });
    expect(screen.getByRole("link", { name: "OpenStreetMap" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "View voyage" }));
    expect(screen.getByTestId("current-path")).toHaveTextContent("/voyages/new-voyage");
    expect(
      screenMocks.upload.mock.calls.every(([request]) => request.voyageId === "new-voyage"),
    ).toBe(true);
  });

  it("removes a permanent failure from the batch", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockRejectedValue(apiError(413, "too_large"));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    await user.upload(input, fileOfSize(26_000_000, "oversized.jpg"));

    expect(await screen.findByText(/Too large/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove oversized.jpg" }));

    expect(screen.queryByText("oversized.jpg")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Uploaded 0 of 0" })).toBeInTheDocument();
  });

  it("keeps details when draft creation fails and allows retry", async () => {
    const user = userEvent.setup();
    screenMocks.createVoyage.mockRejectedValueOnce(new Error("database unavailable"));
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await user.type(screen.getByRole("textbox", { name: /Voyage Name/ }), "Retry trip");
    await user.type(
      screen.getByRole("textbox", { name: /Voyage Description/ }),
      "Retry description",
    );
    await user.click(screen.getByRole("button", { name: "~50 km" }));
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("database unavailable");
    expect(screenMocks.upload).not.toHaveBeenCalled();
    expect(container.querySelector('input[type="file"]')).toBeNull();
    expect(screen.getByRole("textbox", { name: /Voyage Description/ })).toHaveValue(
      "Retry description",
    );
    expect(screen.getByRole("button", { name: "~50 km" })).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "Next: upload photos" }));
    expect(await screen.findByRole("button", { name: "choose files" })).toBeInTheDocument();
  });

  it("can view the draft without uploading photos", async () => {
    const user = userEvent.setup();
    renderScreen(
      <>
        <Upload />
        <CurrentPath />
      </>,
      { auth: authState(ADA) },
    );
    await enterDetails(user);
    await user.click(screen.getByRole("button", { name: "View voyage" }));
    expect(screen.getByTestId("current-path")).toHaveTextContent("/voyages/new-voyage");
    expect(screenMocks.upload).not.toHaveBeenCalled();
  });

  it("shows filenames and upload outcomes without previews for HEIC, JPEG, and PNG", async () => {
    const user = userEvent.setup();
    const createUrl = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:raw-photo");
    const { container } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    const files = [
      new File(["heic"], "phone.heic", { type: "image/heic" }),
      new File(["jpeg"], "camera.jpg", { type: "image/jpeg" }),
      new File(["png"], "picture.png", { type: "image/png" }),
    ];
    await user.upload(container.querySelector<HTMLInputElement>('input[type="file"]')!, files);
    await waitFor(() => expect(screenMocks.upload).toHaveBeenCalledTimes(3));
    expect(await screen.findByRole("heading", { name: "Uploaded 3 of 3" })).toBeInTheDocument();
    for (const file of files) expect(screen.getByText(file.name)).toBeInTheDocument();
    expect(screen.getAllByText("Uploaded", { exact: true })).toHaveLength(3);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector(".pv-upload-thumb")).toBeNull();
    expect(createUrl).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /review/i }));
    expect(container.querySelector("img")).toBeNull();
  });

  it("aborts the pending upload and never starts remaining files on unmount", async () => {
    const user = userEvent.setup();
    screenMocks.upload.mockImplementation(
      ({ signal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
        }),
    );
    const { container, unmount } = renderScreen(<Upload />, { auth: authState(ADA) });
    await enterDetails(user);
    await user.upload(
      container.querySelector<HTMLInputElement>('input[type="file"]')!,
      ["one", "two", "three"].map(
        (name) => new File(["image"], `${name}.jpg`, { type: "image/jpeg" }),
      ),
    );
    const { signal } = screenMocks.upload.mock.calls[0][0];

    unmount();

    expect(signal.aborted).toBe(true);
    await act(async () => {});
    expect(screenMocks.upload).toHaveBeenCalledTimes(1);
  });
});

it("ignores a retry for an item removed from the batch", async () => {
  const { renderHook, act } = await import("@testing-library/react");
  const { useUploadBatch } = await import("../upload/useUploadBatch");
  const { result } = renderHook(() => useUploadBatch({ current: "v1" }, {}));
  act(() => result.current.retry("removed-item"));
  expect(screenMocks.upload).not.toHaveBeenCalled();
  expect(result.current.items).toEqual([]);
});
