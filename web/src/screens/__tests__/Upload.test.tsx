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

import { describe, expect, it } from "vitest";

import { ApiRequestError, NetworkError } from "../../api/client";
import type { Photo } from "../../api/types";
import { reasonFor, tally } from "../Upload";

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

  /** A proxy returning an HTML error page, or any shape that is not ours, must
   *  still produce a row rather than throwing and taking the batch with it. */
  it("produces a reason for an error shape it does not recognise", () => {
    const reason = reasonFor(new Error("something else entirely"), fileOfSize(1000));

    expect(reason.message).toBeTruthy();
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
