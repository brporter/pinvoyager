/** Accepts chosen or dropped files and displays drag feedback; server validation is separate. */

import { useRef, useState } from "react";
import type { DragEvent, ReactNode } from "react";

import { UploadIcon, CircleXIcon } from "./icons";

export type DropzoneState = "idle" | "dragover" | "reject";

export interface DropzoneProps {
  /** Receives every dropped or chosen file, valid or not. */
  onFiles: (files: File[]) => void;
  /** `accept` for the file input. Does not affect what a drop delivers. */
  accept?: string;
  multiple?: boolean;
  disabled?: boolean;
  /** Shown in the idle hint. Text only — the server enforces the real limit. */
  formatsLabel?: string;
  maxSizeLabel?: string;
  /** Replaces the whole idle hint line. */
  hint?: ReactNode;
}

interface DragInfo {
  state: DropzoneState;
  total: number;
  nonImages: number;
}

const IDLE: DragInfo = { state: "idle", total: 0, nonImages: 0 };

/** What the drag is carrying, as far as the browser will say before the drop.
 *  Types are often empty strings mid-drag on some platforms, which is counted
 *  as "unknown, assume fine" rather than as a rejection. */
function inspect(transfer: DataTransfer | null): DragInfo {
  const items = transfer?.items;
  if (items == null || items.length === 0) {
    return { state: "dragover", total: 0, nonImages: 0 };
  }
  let total = 0;
  let nonImages = 0;
  for (const item of Array.from(items)) {
    if (item.kind !== "file") continue;
    total += 1;
    if (item.type !== "" && !item.type.startsWith("image/")) nonImages += 1;
  }
  return { state: nonImages > 0 ? "reject" : "dragover", total, nonImages };
}

export function Dropzone({
  onFiles,
  accept = "image/*",
  multiple = true,
  disabled = false,
  formatsLabel = "JPEG, PNG, WebP",
  maxSizeLabel = "25 MB",
  hint,
}: DropzoneProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [drag, setDrag] = useState<DragInfo>(IDLE);

  // Drag events fire for every descendant, so entering a child looks like
  // leaving the zone. Counting enters against leaves is what stops the state
  // flickering as the pointer crosses the icon and the text.
  const depth = useRef(0);

  function reset() {
    depth.current = 0;
    setDrag(IDLE);
  }

  function handleDragEnter(event: DragEvent<HTMLDivElement>) {
    if (disabled) return;
    event.preventDefault();
    depth.current += 1;
    setDrag(inspect(event.dataTransfer));
  }

  function handleDragOver(event: DragEvent<HTMLDivElement>) {
    if (disabled) return;
    // Without preventDefault the browser refuses the drop entirely.
    event.preventDefault();
    if (depth.current === 0) depth.current = 1;
    setDrag(inspect(event.dataTransfer));
  }

  function handleDragLeave(event: DragEvent<HTMLDivElement>) {
    if (disabled) return;
    event.preventDefault();
    depth.current = Math.max(0, depth.current - 1);
    if (depth.current === 0) setDrag(IDLE);
  }

  function handleDrop(event: DragEvent<HTMLDivElement>) {
    if (disabled) return;
    event.preventDefault();
    reset();
    const files = Array.from(event.dataTransfer?.files ?? []);
    if (files.length > 0) onFiles(files);
  }

  const { state, total, nonImages } = drag;

  return (
    <div
      className="pv-dropzone"
      data-state={state === "idle" ? undefined : state}
      onDragEnter={handleDragEnter}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <div className="pv-dropzone-icon">
        {state === "reject" ? (
          <CircleXIcon size={34} strokeWidth={1.75} />
        ) : (
          <UploadIcon size={34} strokeWidth={1.75} />
        )}
      </div>

      <p className="pv-dropzone-title">
        {state === "reject"
          ? "Only image files are supported"
          : state === "dragover"
            ? `Release to upload ${total} ${total === 1 ? "photo" : "photos"}`
            : "Drop photos here"}
      </p>

      <p className="pv-dropzone-hint">
        {state === "reject"
          ? `${nonImages} of these ${total} files are not images`
          : state === "dragover"
            ? "They will be uploaded to your voyage"
            : (hint ?? (
                <>
                  or{" "}
                  <button
                    type="button"
                    className="pv-btn pv-btn--ghost pv-btn--sm"
                    disabled={disabled}
                    onClick={() => inputRef.current?.click()}
                  >
                    choose files
                  </button>{" "}
                  · {formatsLabel} · up to {maxSizeLabel} each
                </>
              ))}
      </p>

      {/* `hidden` rather than `.pv-sr`: a visually-hidden-but-focusable input
          would put an invisible stop in the tab order next to the button that
          already does the job. The button is the control; this is plumbing. */}
      <input
        ref={inputRef}
        type="file"
        hidden
        tabIndex={-1}
        accept={accept}
        multiple={multiple}
        disabled={disabled}
        onChange={(event) => {
          const files = Array.from(event.target.files ?? []);
          // Reset so choosing the same file twice fires a change event again.
          event.target.value = "";
          if (files.length > 0) onFiles(files);
        }}
      />
    </div>
  );
}
