/**
 * UploadRow — from `design/components/upload.html`.
 *
 * One file, one row, one outcome. Uploads are batches and batches partially
 * fail, so the row's job is making a mixed result legible rather than
 * pretending the batch succeeds or fails as a unit.
 *
 * Note the two distinct post-upload states the design draws: a photo with
 * coordinates shows them, and one without shows the "No location" badge and the
 * words "needs placing". The difference is visible in the list rather than
 * discovered later on an empty map — which is why `unplaced` is a separate flag
 * from `status`, not a kind of failure.
 *
 * The progress bar is a real `progressbar` with `aria-valuenow`. The design
 * draws a bare div, but a progress indicator that only exists visually tells a
 * screen-reader user nothing about whether their upload is moving.
 */

import type { ReactNode } from "react";

import { Badge } from "./Badge";
import { Button, Spinner } from "./Button";
import { cx } from "./cx";
import { CheckIcon, CloseIcon, ErrorCircleIcon } from "./icons";

export type UploadStatus =
  | "queued"
  | "uploading"
  | "processing"
  | "uploaded"
  | "error";

export interface UploadRowProps {
  /** The file name, as the user knows it. */
  name: string;
  status: UploadStatus;
  /** 0–100. Ignored unless the status is `uploading` or `processing`. */
  progress?: number;
  /** The line under the name — "4.2 MB · uploading", "2.1 MB · 43.06°, 141.35°".
   *  Ignored when `status` is `error`; the error message takes that line. */
  detail?: ReactNode;
  /** Why the file failed, e.g. "Too large — 68 MB, limit is 25 MB". */
  error?: string;
  /** Uploaded, but with no GPS EXIF. Adds the "No location" badge. */
  unplaced?: boolean;
  /** Local object URL or a server thumbnail. Absent for a failed file, which
   *  gets the error glyph in the thumbnail slot instead. */
  thumbnailUrl?: string;
  onCancel?: () => void;
  onRemove?: () => void;
  onRetry?: () => void;
  className?: string;
}

export function UploadRow({
  name,
  status,
  progress,
  detail,
  error,
  unplaced = false,
  thumbnailUrl,
  onCancel,
  onRemove,
  onRetry,
  className,
}: UploadRowProps) {
  const failed = status === "error";
  const showProgress = status === "uploading" || status === "processing";
  const value = status === "processing" ? 100 : Math.round(progress ?? 0);

  return (
    <div className={cx("pv-upload-row", className)}>
      <div className="pv-upload-thumb">
        {failed ? (
          <ErrorCircleIcon size={18} strokeWidth={2} />
        ) : thumbnailUrl != null ? (
          <img className="pv-photo-img" src={thumbnailUrl} alt="" />
        ) : null}
      </div>

      <div className="pv-upload-info">
        <div className="pv-upload-name">{name}</div>

        <div className={cx("pv-upload-sub", failed && "pv-upload-sub--error")}>
          {unplaced && !failed && <Badge tone="unplaced">No location</Badge>}
          {failed ? error : detail}
        </div>

        {showProgress && (
          <div
            className="pv-progress"
            role="progressbar"
            aria-label={`Upload progress for ${name}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={value}
          >
            <div className="pv-progress-bar" style={{ width: `${value}%` }} />
          </div>
        )}
      </div>

      {status === "uploading" && onCancel != null && (
        <Button
          variant="ghost"
          size="sm"
          iconOnly
          aria-label={`Cancel upload of ${name}`}
          onClick={onCancel}
        >
          <CloseIcon size={14} />
        </Button>
      )}

      {status === "processing" && <Spinner />}

      {status === "uploaded" && (
        <>
          <CheckIcon size={18} strokeWidth={2.5} stroke="var(--pv-success)" />
          {/* The tick is the only thing distinguishing a finished row from a
              queued one, so its meaning is spelled out for anyone who cannot
              see it. */}
          <span className="pv-sr">Uploaded</span>
        </>
      )}

      {failed && onRetry != null && (
        <Button variant="ghost" size="sm" onClick={onRetry}>
          Retry
        </Button>
      )}

      {failed && onRemove != null && (
        <Button
          variant="ghost"
          size="sm"
          iconOnly
          aria-label={`Remove ${name}`}
          onClick={onRemove}
        >
          <CloseIcon size={14} />
        </Button>
      )}
    </div>
  );
}
