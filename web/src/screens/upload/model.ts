import { isApiError, NetworkError } from "../../api/client";
import type { Photo } from "../../api/types";

/** Why a file failed, in the two parts a row needs: what to say, and whether
 *  sending the same bytes again could possibly help. */
interface Failure {
  message: string;
  retryable: boolean;
}

export type ItemState =
  | {
      status: "queued" | "uploading" | "processing";
      progress: number;
      photo?: never;
      failure?: never;
    }
  | { status: "uploaded"; progress: number; photo: Photo; failure?: never }
  | { status: "error"; progress: number; failure: Failure; photo?: never };

export type Item = { id: string; file: File } & ItemState;

export const MAX_SIZE_LABEL = "25 MB";

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

export function megabytes(bytes: number): string {
  const mb = bytes / 1_000_000;
  return mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

export function coordinateLabel(photo: Photo): string | null {
  if (photo.latitude === null || photo.longitude === null) return null;
  return `${photo.latitude.toFixed(2)}°, ${photo.longitude.toFixed(2)}°`;
}

export function plural(count: number, one: string, many: string): string {
  return count === 1 ? one : many;
}

/**
 * The sentence a failed row shows.
 *
 * Switches on `code`, which is the stable half of an error body. The server's
 * `message` is prose and is used only where no code applies — an unexpected
 * status, or a proxy answering with something that is not our error shape at
 * all.
 */
export function reasonFor(error: unknown, file: File): Failure {
  if (error instanceof NetworkError) {
    return {
      message: "Could not be sent — check your connection",
      retryable: true,
    };
  }

  if (!isApiError(error)) {
    return { message: "Upload failed for an unknown reason", retryable: true };
  }

  switch (error.code) {
    case "too_large":
      // The design's wording, with the real number in it: a limit stated
      // without the file's own size leaves the user guessing how far over.
      return {
        message: `Too large — ${megabytes(file.size)}, limit is ${MAX_SIZE_LABEL}`,
        retryable: false,
      };

    case "not_an_image":
      // The server decides this by magic bytes, so it fires for a .jpg whose
      // contents are something else — not only for an obvious .pdf.
      return { message: "Not an image file — skipped", retryable: false };

    case "unprocessable_image":
      return {
        message: "Image dimensions are beyond the limit — skipped",
        retryable: false,
      };

    case "rate_limited": {
      const wait = error.retryAfterSeconds;
      return {
        message:
          wait === null
            ? "Too many uploads just now — try this one again in a moment"
            : `Too many uploads just now — try this one again in ${Math.ceil(wait)} seconds`,
        retryable: true,
      };
    }

    case "unauthorized":
      return {
        message: "Your session ended — sign in again to upload this file",
        retryable: false,
      };

    default:
      return {
        message:
          error.status >= 500
            ? `The server failed on this file${error.requestId === null ? "" : ` — reference ${error.requestId}`}`
            : error.message,
        retryable: error.status >= 500,
      };
  }
}

// ---------------------------------------------------------------------------
// Counting
// ---------------------------------------------------------------------------

export interface BatchTally {
  total: number;
  uploaded: number;
  unplaced: number;
  failed: number;
  pending: number;
}

/** Every outcome, including the failures. This is the number the summary
 *  reports, and the reason it is computed in one place is that a second count
 *  written beside the first is how "4 of 6" and "2 skipped" drift apart. */
export function tally(items: readonly Item[]): BatchTally {
  let uploaded = 0;
  let unplaced = 0;
  let failed = 0;
  let pending = 0;

  for (const item of items) {
    if (item.status === "uploaded") {
      uploaded += 1;
      if (item.photo.latitude == null) unplaced += 1;
    } else if (item.status === "error") {
      failed += 1;
    } else {
      pending += 1;
    }
  }

  return { total: items.length, uploaded, unplaced, failed, pending };
}
