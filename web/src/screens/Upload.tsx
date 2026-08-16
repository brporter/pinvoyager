/**
 * Upload — implements `design/screens/upload-flow.html` (T8).
 *
 * Three steps: choose, process, review.
 *
 * ## The thing this screen exists to do
 *
 * A real batch is mixed. Some files upload, some carry no location, and some
 * the server refuses — and the design's whole argument is that each of those
 * outcomes belongs on its own row rather than collapsed into one batch-level
 * verdict. Two consequences run through the code below:
 *
 *  * **One request per file.** `uploadPhotos` accepts many, but a rejected file
 *    fails the request that carried it, so a single request for six files would
 *    turn one bad file into six unknown outcomes. Uploading them one at a time
 *    is what gives each file its own status, its own progress and its own
 *    reason for failing.
 *  * **The summary counts everything.** "4 of 6 photos uploaded · 1 has no
 *    location · 2 were skipped". A summary that reported only successes would
 *    let someone walk away believing a batch completed when two files never
 *    made it.
 *
 * ## Branching on `code`, not on prose
 *
 * The server's refusals arrive as {@link ApiRequestError} with a stable `code`:
 * `too_large` (413), `not_an_image` (415), `unprocessable_image` (422) and
 * `rate_limited` (429). {@link reasonFor} switches on that rather than on
 * `message`, which is prose written for a person and may be reworded at any
 * time.
 *
 * ## Why step three exists
 *
 * EXIF coordinates can be wrong — a camera holding a stale GPS fix will place a
 * photograph hundreds of kilometres from where it was taken — and the review
 * map is the only chance to notice before anything is published. Photos with no
 * location get an explicit decision there rather than a silent default, and the
 * primary button counts what will actually happen: "Add 4 photos", not "Done".
 */

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";

import { attachPhotos, createVoyage, isApiError, NetworkError } from "../api/client";
import { keys, useUploadPhotos, useVoyages } from "../api/hooks";
import type { Photo, VoyageSummary } from "../api/types";
import { useAuth } from "../auth/useAuth";
import {
  Attribution,
  Badge,
  Button,
  Checkbox,
  Dropzone,
  EmptyState,
  ImageIcon,
  Pin,
  Select,
  ShieldCheckIcon,
  Skeleton,
  TextField,
  Toast,
  UploadRow,
} from "../components";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

type Step = "choose" | "process" | "review";

type ItemStatus = "queued" | "uploading" | "processing" | "uploaded" | "error";

/** Why a file failed, in the two parts a row needs: what to say, and whether
 *  sending the same bytes again could possibly help. */
interface Failure {
  message: string;
  retryable: boolean;
}

interface Item {
  /** Stable across retries, so the row does not remount and lose its place. */
  id: string;
  file: File;
  status: ItemStatus;
  /** 0–100, from the XHR upload progress event. */
  progress: number;
  /** Local object URL, so a thumbnail exists before the server has one. */
  thumbnailUrl?: string;
  /** Set once the server accepts the file. Carries the derived coordinates. */
  photo?: Photo;
  failure?: Failure;
}

/** What the review step will do when the primary button is pressed. */
type Destination =
  | { kind: "voyage"; voyageId: string }
  | { kind: "new" }
  | { kind: "library" };

const MAX_SIZE_LABEL = "25 MB";

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

function megabytes(bytes: number): string {
  const mb = bytes / 1_000_000;
  return mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

function coordinateLabel(photo: Photo): string | null {
  if (photo.latitude === null || photo.longitude === null) return null;
  return `${photo.latitude.toFixed(2)}°, ${photo.longitude.toFixed(2)}°`;
}

function plural(count: number, one: string, many: string): string {
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
      if (item.photo?.latitude == null) unplaced += 1;
    } else if (item.status === "error") {
      failed += 1;
    } else {
      pending += 1;
    }
  }

  return { total: items.length, uploaded, unplaced, failed, pending };
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

export function Upload() {
  const navigate = useNavigate();
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const upload = useUploadPhotos();

  const [step, setStep] = useState<Step>("choose");
  const [items, setItems] = useState<Item[]>([]);

  /** One controller per in-flight file, so a single row's cancel button
   *  abandons that upload and nothing else. */
  const controllers = useRef(new Map<string, AbortController>());
  /** Object URLs outlive their component, so they are tracked here and revoked
   *  on unmount rather than leaked for the life of the tab. */
  const objectUrls = useRef<string[]>([]);
  const counter = useRef(0);

  useEffect(() => {
    const inFlight = controllers.current;
    const urls = objectUrls.current;
    return () => {
      for (const controller of inFlight.values()) controller.abort();
      for (const url of urls) revokeObjectUrl(url);
    };
  }, []);

  const patch = useCallback((id: string, change: Partial<Item>) => {
    setItems((current) =>
      current.map((item) => (item.id === id ? { ...item, ...change } : item)),
    );
  }, []);

  const { mutateAsync: send } = upload;

  const uploadOne = useCallback(
    async (item: Item): Promise<void> => {
      const controller = new AbortController();
      controllers.current.set(item.id, controller);
      patch(item.id, { status: "uploading", progress: 0, failure: undefined });

      try {
        const result = await send({
          files: [item.file],
          signal: controller.signal,
          onProgress: (fraction) => {
            if (fraction === null) {
              patch(item.id, { status: "processing", progress: 100 });
              return;
            }
            const percent = Math.round(fraction * 100);
            // The bytes are gone but the response has not come back: the
            // server is decoding the header and reading EXIF. The design draws
            // that as its own state — "reading location data" — rather than a
            // bar that sits at 100% looking stuck.
            patch(
              item.id,
              percent >= 100
                ? { status: "processing", progress: 100 }
                : { status: "uploading", progress: percent },
            );
          },
        });

        const photo = result.photos[0];
        if (photo === undefined) {
          patch(item.id, {
            status: "error",
            failure: {
              message: "The server accepted the file but returned no photo",
              retryable: true,
            },
          });
          return;
        }

        patch(item.id, {
          status: "uploaded",
          progress: 100,
          photo,
          failure: undefined,
        });
      } catch (error) {
        patch(item.id, {
          status: "error",
          failure: controller.signal.aborted
            ? { message: "Cancelled", retryable: true }
            : reasonFor(error, item.file),
        });
      } finally {
        controllers.current.delete(item.id);
      }
    },
    [patch, send],
  );

  /** Sequential on purpose. Six parallel uploads race the per-user rate
   *  limiter into a 429 that has nothing to do with the files, and the design
   *  draws one row in flight at a time. */
  const runBatch = useCallback(
    async (batch: readonly Item[]): Promise<void> => {
      for (const item of batch) {
        await uploadOne(item);
      }
    },
    [uploadOne],
  );

  const onFiles = useCallback(
    (files: File[]) => {
      const added = files.map((file) => {
        counter.current += 1;
        const url = createObjectUrl(file);
        if (url !== undefined) objectUrls.current.push(url);
        return {
          id: `f${String(counter.current)}`,
          file,
          status: "queued" as const,
          progress: 0,
          thumbnailUrl: url,
        };
      });

      setItems((current) => [...current, ...added]);
      setStep("process");
      void runBatch(added);
    },
    [runBatch],
  );

  const counts = tally(items);

  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-6)" }}>
      <div style={{ maxWidth: 720, margin: "0 auto" }}>
        <h1 className="pv-h2" style={{ marginBottom: "var(--pv-space-5)" }}>
          {headingFor(step, counts)}
        </h1>

        <Steps current={step} />

        {step === "choose" && <ChooseStep onFiles={onFiles} />}

        {step === "process" && (
          <ProcessStep
            items={items}
            counts={counts}
            onCancel={(id) => controllers.current.get(id)?.abort()}
            onRetry={(id) => {
              const item = items.find((candidate) => candidate.id === id);
              if (item !== undefined) void uploadOne(item);
            }}
            onRemove={(id) =>
              setItems((current) => current.filter((item) => item.id !== id))
            }
            onReview={() => setStep("review")}
            onStartOver={() => {
              setItems([]);
              setStep("choose");
            }}
          />
        )}

        {step === "review" && (
          <ReviewStep
            items={items}
            counts={counts}
            onBack={() => setStep("process")}
            onDone={(voyageId) => {
              void queryClient.invalidateQueries({ queryKey: keys.photos.all });
              void queryClient.invalidateQueries({ queryKey: keys.voyages.all });
              void navigate(
                voyageId === null
                  ? user === null
                    ? "/"
                    : `/u/${user.id}`
                  : `/voyages/${voyageId}`,
              );
            }}
          />
        )}
      </div>
    </div>
  );
}

function headingFor(step: Step, counts: BatchTally): string {
  if (step === "choose") return "Upload photos";
  if (step === "review") return "Review";
  return counts.pending > 0
    ? `Uploading ${String(counts.total)} ${plural(counts.total, "photo", "photos")}`
    : `Uploaded ${String(counts.uploaded)} of ${String(counts.total)}`;
}

// ---------------------------------------------------------------------------
// The step indicator
// ---------------------------------------------------------------------------

const STEP_ORDER: readonly Step[] = ["choose", "process", "review"];
const STEP_LABEL: Record<Step, string> = {
  choose: "Choose",
  process: "Process",
  review: "Review",
};

/**
 * Choose · Process · Review.
 *
 * The design file styles this with rules local to that file rather than with
 * anything in `design/components.css`, so the geometry is mirrored inline here.
 * Nothing new is invented — the values are the ones drawn — but if this pattern
 * appears on a second screen it belongs in the design system rather than in two
 * copies of these objects.
 */
function Steps({ current }: { current: Step }) {
  const index = STEP_ORDER.indexOf(current);

  return (
    <ol
      aria-label="Upload progress"
      style={{
        display: "flex",
        gap: "var(--pv-space-2)",
        alignItems: "center",
        margin: "0 0 var(--pv-space-6)",
        padding: 0,
        listStyle: "none",
        flexWrap: "wrap",
      }}
    >
      {STEP_ORDER.map((step, position) => {
        const state =
          position < index ? "done" : position === index ? "current" : "todo";

        return (
          <li
            key={step}
            aria-current={state === "current" ? "step" : undefined}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--pv-space-2)",
              fontSize: "var(--pv-text-sm)",
              color:
                state === "todo" ? "var(--pv-text-subtle)" : "var(--pv-text)",
            }}
          >
            <span
              aria-hidden="true"
              style={{
                width: 22,
                height: 22,
                flex: "none",
                borderRadius: "var(--pv-radius-full)",
                display: "grid",
                placeItems: "center",
                fontSize: "var(--pv-text-2xs)",
                fontWeight: "var(--pv-weight-bold)",
                background:
                  state === "done"
                    ? "var(--pv-success)"
                    : state === "current"
                      ? "var(--pv-accent)"
                      : "var(--pv-bg-sunken)",
                color:
                  state === "done"
                    ? "var(--pv-neutral-0)"
                    : state === "current"
                      ? "var(--pv-text-on-accent)"
                      : "var(--pv-text-subtle)",
              }}
            >
              {state === "done" ? "✓" : position + 1}
            </span>
            {STEP_LABEL[step]}
            {/* Announced so the list does not read as three bare words with no
                indication of which one the user is on. */}
            {state === "done" && <span className="pv-sr">(done)</span>}
          </li>
        );
      })}
    </ol>
  );
}

// ---------------------------------------------------------------------------
// Step 1 — choose
// ---------------------------------------------------------------------------

function ChooseStep({ onFiles }: { onFiles: (files: File[]) => void }) {
  return (
    <>
      <Dropzone onFiles={onFiles} maxSizeLabel={MAX_SIZE_LABEL} />

      {/* The stripping guarantee appears here as well as at the granularity
          control. This is the moment someone decides whether to trust the
          product with their camera roll, so it is the moment it is worth the
          space. */}
      <div
        className="pv-gran-guarantee"
        style={{ marginTop: "var(--pv-space-5)" }}
      >
        <ShieldCheckIcon size={16} />
        <div>
          <strong>Your originals are never published.</strong> PinVoyager reads
          the location and time from each photo, then serves re-encoded copies
          with all metadata removed.
        </div>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Step 2 — process
// ---------------------------------------------------------------------------

function ProcessStep({
  items,
  counts,
  onCancel,
  onRetry,
  onRemove,
  onReview,
  onStartOver,
}: {
  items: readonly Item[];
  counts: BatchTally;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
  onRemove: (id: string) => void;
  onReview: () => void;
  onStartOver: () => void;
}) {
  const settled = counts.pending === 0;

  return (
    <>
      <div className="pv-card" aria-busy={!settled}>
        {items.map((item) => (
          <UploadRow
            key={item.id}
            name={item.file.name}
            status={item.status}
            progress={item.progress}
            thumbnailUrl={item.thumbnailUrl}
            unplaced={item.status === "uploaded" && item.photo?.latitude == null}
            detail={detailFor(item)}
            error={item.failure?.message}
            onCancel={item.status === "uploading" ? () => onCancel(item.id) : undefined}
            onRetry={
              item.status === "error" && item.failure?.retryable === true
                ? () => onRetry(item.id)
                : undefined
            }
            onRemove={item.status === "error" ? () => onRemove(item.id) : undefined}
          />
        ))}
      </div>

      {/* Live so the outcome is announced as it settles rather than only being
          visible. `polite` because it changes several times per batch. */}
      <div aria-live="polite" style={{ marginTop: "var(--pv-space-4)" }}>
        {settled && <Summary counts={counts} />}
      </div>

      {settled && (
        <div
          className="pv-inline-3"
          style={{ marginTop: "var(--pv-space-4)", justifyContent: "flex-end" }}
        >
          <Button variant="ghost" onClick={onStartOver}>
            Choose different files
          </Button>
          {counts.uploaded > 0 && (
            <Button variant="primary" onClick={onReview}>
              Review {counts.uploaded}{" "}
              {plural(counts.uploaded, "photo", "photos")}
            </Button>
          )}
        </div>
      )}

      {settled && counts.uploaded === 0 && (
        <EmptyState
          icon={<ImageIcon size={40} />}
          tone="danger"
          title="Nothing was uploaded"
          className="pv-card"
        >
          Every file in this batch was refused. The reason for each one is on its
          row above.
        </EmptyState>
      )}
    </>
  );
}

function detailFor(item: Item): string {
  const size = megabytes(item.file.size);

  switch (item.status) {
    case "queued":
      return `${size} · waiting`;
    case "uploading":
      return `${size} · uploading`;
    case "processing":
      return `${size} · reading location data`;
    case "uploaded": {
      const at = item.photo === undefined ? null : coordinateLabel(item.photo);
      return at === null ? `${size} · uploaded, needs placing` : `${size} · ${at}`;
    }
    case "error":
      return size;
  }
}

/**
 * The batch summary — every outcome, failures included.
 *
 * "4 of 6 photos uploaded" on its own would let someone walk away believing the
 * batch completed. The second line is the part that makes the mixed result
 * legible, so there is no variant of this component without it.
 */
function Summary({ counts }: { counts: BatchTally }) {
  const notes: string[] = [];
  if (counts.unplaced > 0) {
    notes.push(
      `${String(counts.unplaced)} ${plural(counts.unplaced, "has", "have")} no location`,
    );
  }
  if (counts.failed > 0) {
    notes.push(
      `${String(counts.failed)} ${plural(counts.failed, "was", "were")} skipped`,
    );
  }

  return (
    <div className="pv-card" style={{ padding: "var(--pv-space-4)" }}>
      <div style={{ fontWeight: "var(--pv-weight-semibold)" }}>
        {counts.uploaded} of {counts.total}{" "}
        {plural(counts.total, "photo", "photos")} uploaded
      </div>
      <div className="pv-xs pv-muted">
        {notes.length === 0 ? "Every file made it." : notes.join(" · ")}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 3 — review
// ---------------------------------------------------------------------------

function ReviewStep({
  items,
  counts,
  onBack,
  onDone,
}: {
  items: readonly Item[];
  counts: BatchTally;
  onBack: () => void;
  /** `null` means the photos were left in the library. */
  onDone: (voyageId: string | null) => void;
}) {
  const voyages = useVoyages({ mine: true });

  const uploaded = useMemo(
    () =>
      items
        .map((item) => item.photo)
        .filter((photo): photo is Photo => photo !== undefined),
    [items],
  );
  const placed = uploaded.filter((photo) => photo.latitude !== null);
  const unplaced = uploaded.filter((photo) => photo.latitude === null);

  const [destination, setDestination] = useState<Destination | null>(null);
  const [title, setTitle] = useState("");
  const [includeUnplaced, setIncludeUnplaced] = useState(true);

  const options: VoyageSummary[] = useMemo(
    () => voyages.data?.pages.flatMap((page) => page.voyages) ?? [],
    [voyages.data],
  );

  // Chosen once the list arrives, and only if the user has not chosen already.
  // "Leave in my library" is the fallback rather than "create a voyage",
  // because a default that creates something is a default that acts.
  useEffect(() => {
    if (destination !== null || voyages.isPending) return;
    const first = options[0];
    setDestination(
      first === undefined ? { kind: "library" } : { kind: "voyage", voyageId: first.id },
    );
  }, [destination, options, voyages.isPending]);

  const chosen: Destination = destination ?? { kind: "library" };
  const toAttach =
    chosen.kind === "library"
      ? uploaded
      : includeUnplaced
        ? uploaded
        : placed;

  const finish = useMutation({
    mutationFn: async (): Promise<string | null> => {
      if (chosen.kind === "library") return null;

      const voyageId =
        chosen.kind === "new"
          ? (await createVoyage(title.trim() === "" ? {} : { title: title.trim() })).id
          : chosen.voyageId;

      if (toAttach.length > 0) {
        await attachPhotos(
          voyageId,
          toAttach.map((photo) => photo.id),
        );
      }
      return voyageId;
    },
    retry: false,
    onSuccess: onDone,
  });

  return (
    <>
      <Summary counts={counts} />

      <ReviewMap photos={placed} unplacedCount={unplaced.length} />

      <div style={{ marginTop: "var(--pv-space-4)" }}>
        {voyages.isPending ? (
          <div aria-busy="true">
            <span className="pv-sr">Loading your voyages…</span>
            <Skeleton height={18} width="40%" />
            <Skeleton height={44} />
          </div>
        ) : voyages.isError ? (
          <Toast tone="error" title="Your voyages could not be loaded">
            You can still leave these photos in your library and add them later.{" "}
            <Button variant="ghost" size="sm" onClick={() => void voyages.refetch()}>
              Try again
            </Button>
          </Toast>
        ) : (
          <Select
            label="Add to voyage"
            value={valueOf(chosen)}
            help={
              options.length === 0
                ? "You have no voyages yet — create one, or keep these in your library for now."
                : undefined
            }
            onChange={(event) => setDestination(destinationOf(event.target.value))}
          >
            {options.map((voyage) => (
              <option key={voyage.id} value={`voyage:${voyage.id}`}>
                {voyage.title}
                {voyage.published ? "" : " (draft)"}
              </option>
            ))}
            <option value="new">Create a new voyage…</option>
            {/* Uploading and assigning are separate acts. Forcing every upload
                into a voyage immediately would make the library meaningless. */}
            <option value="library">Leave in my library</option>
          </Select>
        )}
      </div>

      {chosen.kind === "new" && (
        <div style={{ marginTop: "var(--pv-space-4)" }}>
          <TextField
            label="Voyage title"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            help="You can change this later. Leave it blank and the voyage is created untitled."
          />
        </div>
      )}

      {chosen.kind !== "library" && unplaced.length > 0 && (
        <div
          className="pv-card"
          style={{
            padding: "var(--pv-space-3)",
            background: "var(--pv-warning-bg)",
            borderColor: "transparent",
            marginTop: "var(--pv-space-4)",
          }}
        >
          <div
            className="pv-sm"
            style={{ fontWeight: "var(--pv-weight-semibold)", marginBottom: 2 }}
          >
            {unplaced.length} {plural(unplaced.length, "photo has", "photos have")} no
            location
          </div>
          <div className="pv-xs pv-muted">
            {plural(unplaced.length, "It", "They")} will be part of the voyage but
            will not appear on the map.
          </div>
          <div style={{ marginTop: "var(--pv-space-3)" }}>
            <Checkbox
              checked={includeUnplaced}
              onChange={(event) => setIncludeUnplaced(event.target.checked)}
            >
              Include {plural(unplaced.length, "it", "them")} anyway
            </Checkbox>
          </div>
        </div>
      )}

      {finish.isError && (
        <div style={{ marginTop: "var(--pv-space-4)" }}>
          <Toast tone="error" title="Those photos could not be added">
            {finish.error.message} Your photos are safe in your library — nothing
            was lost.
          </Toast>
        </div>
      )}

      <div
        className="pv-inline-3"
        style={{ marginTop: "var(--pv-space-5)", justifyContent: "flex-end" }}
      >
        <Button variant="ghost" onClick={onBack}>
          Back
        </Button>
        {/* Counts what will actually happen, not "Done". Six were selected;
            four are being added. */}
        <Button
          variant="primary"
          size="lg"
          loading={finish.isPending}
          onClick={() => void finish.mutateAsync().catch(() => undefined)}
        >
          {chosen.kind === "library"
            ? `Keep ${String(toAttach.length)} ${plural(toAttach.length, "photo", "photos")} in my library`
            : `Add ${String(toAttach.length)} ${plural(toAttach.length, "photo", "photos")}`}
        </Button>
      </div>
    </>
  );
}

function valueOf(destination: Destination): string {
  switch (destination.kind) {
    case "voyage":
      return `voyage:${destination.voyageId}`;
    case "new":
      return "new";
    case "library":
      return "library";
  }
}

function destinationOf(value: string): Destination {
  if (value === "new") return { kind: "new" };
  if (value.startsWith("voyage:")) {
    return { kind: "voyage", voyageId: value.slice("voyage:".length) };
  }
  return { kind: "library" };
}

/**
 * Where the photographs actually landed.
 *
 * A placeholder at the dimensions the design draws, filled in by phase 7. The
 * pins are positioned by a linear fit of the batch's own bounding box, which is
 * not a projection and is not pretending to be one — what it has to show is
 * that three photos are near each other and one is not, which is exactly the
 * mistake this step exists to catch.
 */
function ReviewMap({
  photos,
  unplacedCount,
}: {
  photos: readonly Photo[];
  unplacedCount: number;
}) {
  const positions = useMemo(() => layout(photos), [photos]);

  return (
    <div style={{ marginTop: "var(--pv-space-4)" }}>
      <div
        className="pv-map"
        style={{
          height: "clamp(180px, 34vw, 300px)",
          borderRadius: "var(--pv-radius-lg)",
          position: "relative",
        }}
        role="group"
        aria-label={`Where these photos landed — ${String(photos.length)} on the map`}
      >
        {positions.map(({ photo, left, top }) => (
          <Pin
            key={photo.id}
            left={left}
            top={top}
            label={`A photo at ${coordinateLabel(photo) ?? "an unknown position"}`}
          />
        ))}
        <Attribution />
      </div>

      <p className="pv-xs pv-muted" style={{ marginTop: "var(--pv-space-2)" }}>
        Check this before publishing. A camera holding an old GPS fix can place a
        photo hundreds of kilometres from where it was taken.
        {unplacedCount > 0 && (
          <>
            {" "}
            <Badge tone="unplaced">No location</Badge> {unplacedCount}{" "}
            {plural(unplacedCount, "photo is", "photos are")} not on the map.
          </>
        )}
      </p>
    </div>
  );
}

interface Placement {
  photo: Photo;
  left: string;
  top: string;
}

function layout(photos: readonly Photo[]): Placement[] {
  const located = photos.filter(
    (photo): photo is Photo & { latitude: number; longitude: number } =>
      photo.latitude !== null && photo.longitude !== null,
  );
  if (located.length === 0) return [];

  const lats = located.map((photo) => photo.latitude);
  const lons = located.map((photo) => photo.longitude);
  const minLat = Math.min(...lats);
  const maxLat = Math.max(...lats);
  const minLon = Math.min(...lons);
  const maxLon = Math.max(...lons);

  const scale = (value: number, low: number, high: number): number =>
    high - low < 1e-9 ? 50 : 15 + ((value - low) / (high - low)) * 70;

  return located.map((photo) => ({
    photo,
    left: `${scale(photo.longitude, minLon, maxLon).toFixed(1)}%`,
    // North is up, so latitude runs the other way down the box.
    top: `${(100 - scale(photo.latitude, minLat, maxLat)).toFixed(1)}%`,
  }));
}

// ---------------------------------------------------------------------------
// Object URLs
// ---------------------------------------------------------------------------

/** jsdom has no `createObjectURL`, and a screen that throws in its test
 *  environment is a screen with no tests. A missing thumbnail is a state
 *  `UploadRow` already draws. */
function createObjectUrl(file: File): string | undefined {
  if (typeof URL.createObjectURL !== "function") return undefined;
  try {
    return URL.createObjectURL(file);
  } catch {
    return undefined;
  }
}

function revokeObjectUrl(url: string): void {
  if (typeof URL.revokeObjectURL !== "function") return;
  try {
    URL.revokeObjectURL(url);
  } catch {
    // Already revoked, or an environment without the API. Nothing to do.
  }
}
