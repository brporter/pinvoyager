/**
 * Upload — implements `design/screens/upload-flow.html` (T8).
 *
 * Enter voyage details, choose photos, upload directly to its draft, then review.
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
 * photograph hundreds of kilometers from where it was taken — and the review
 * map is the only chance to notice before anything is published. Photos with no
 * location are counted explicitly and stay in the voyage. All uploaded
 * photos already belong to the draft; review opens the voyage without a later
 * library-to-voyage assignment.
 */

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";

import { createVoyage, isApiError, NetworkError } from "../api/client";
import { keys, useUploadPhotos } from "../api/hooks";
import type { Photo } from "../api/types";
import {
  Attribution,
  Badge,
  Button,
  Dropzone,
  EmptyState,
  ImageIcon,
  Pin,
  ShieldCheckIcon,
  TextField,
  TextArea,
  Toast,
  UploadRow,
} from "../components";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

type Step = "details" | "choose" | "process" | "review";

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
  const [search] = useSearchParams();
  const [voyageId, setVoyageId] = useState<string | null>(search.get("voyage"));
  const destination = useRef(voyageId);
  const [title, setTitle] = useState("");
  const [nameError, setNameError] = useState<string>();
  const [description, setDescription] = useState("");
  const [descriptionError, setDescriptionError] = useState<string>();
  const queryClient = useQueryClient();
  const upload = useUploadPhotos();

  const [step, setStep] = useState<Step>(voyageId === null ? "details" : "choose");
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
          voyageId: destination.current!,
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
            ? { message: "Canceled", retryable: true }
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

  const create = useMutation({
    mutationFn: async () => {
      const voyage = await createVoyage({ title: title.trim(), description: description.trim() });
      destination.current = voyage.id;
      setVoyageId(voyage.id);
      setStep("choose");
    },
    retry: false,
  });

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
          {step === "details" ? "Create voyage" : step === "choose" ? "Add photos" : headingFor(step, counts)}
        </h1>

        <Steps current={step} />

        {step === "details" && (
          <form className="pv-stack-6" noValidate onSubmit={(event) => {
            event.preventDefault();
            const missingName = title.trim() === "";
            const missingDescription = description.trim() === "";
            setNameError(missingName ? "Enter a voyage name." : undefined);
            setDescriptionError(missingDescription ? "Enter a voyage description." : undefined);
            if (missingName || missingDescription) return;
            create.mutate();
          }}>
            <p className="pv-sm pv-muted" style={{ margin: 0 }}>
              You will be able to upload your photos at the next step.
            </p>
            <TextField label="Voyage Name" required value={title} error={nameError}
              disabled={create.isPending} onChange={(event) => {
                setTitle(event.target.value);
                setNameError(undefined);
              }} />
            <TextArea label="Voyage Description" required value={description} error={descriptionError}
              rows={4} disabled={create.isPending} onChange={(event) => {
                setDescription(event.target.value);
                setDescriptionError(undefined);
              }} />
            {create.isError && (
              <Toast tone="error" title="The voyage could not be created">{create.error.message}</Toast>
            )}
            <div className="pv-inline-3" style={{ justifyContent: "flex-end" }}>
              <Button type="submit" variant="primary" size="lg" loading={create.isPending} disabled={create.isPending}>
                Next: upload photos
              </Button>
            </div>
          </form>
        )}

        {step === "choose" && (
          <div className="pv-stack-6">
            <ChooseStep onFiles={onFiles} disabled={false} />
            <div className="pv-inline-3" style={{ justifyContent: "flex-end" }}>
              <Button variant="ghost" onClick={() => void navigate(`/voyages/${voyageId}`)}>
                View voyage
              </Button>
            </div>
          </div>
        )}

        {step === "process" && (
          <ProcessStep
            items={items}
            counts={counts}
            onCancel={(id) => controllers.current.get(id)?.abort()}
            onRetry={(id) => {
              // This handler comes from the row for this item, so the item is
              // necessarily in the same render's batch.
              void uploadOne(items.find((candidate) => candidate.id === id)!);
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
            onDone={() => {
              void queryClient.invalidateQueries({ queryKey: keys.voyages.all });
              void navigate(`/voyages/${voyageId}`);
            }}
          />
        )}
      </div>
    </div>
  );
}

function headingFor(step: Exclude<Step, "details" | "choose">, counts: BatchTally): string {
  if (step === "review") return "Review";
  return counts.pending > 0
    ? `Uploading ${String(counts.total)} ${plural(counts.total, "photo", "photos")}`
    : `Uploaded ${String(counts.uploaded)} of ${String(counts.total)}`;
}

// ---------------------------------------------------------------------------
// The step indicator
// ---------------------------------------------------------------------------

const STEP_ORDER: readonly Step[] = ["details", "choose", "process", "review"];
const STEP_LABEL: Record<Step, string> = {
  details: "Voyage details",
  choose: "Upload photos",
  process: "Process",
  review: "Review",
};

/**
 * Voyage details · Upload photos · Process · Review.
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

function ChooseStep({ onFiles, disabled }: { onFiles: (files: File[]) => void; disabled: boolean }) {
  return (
    <div>
      <Dropzone onFiles={onFiles} disabled={disabled} formatsLabel="JPEG, PNG, WebP, HEIC" maxSizeLabel={MAX_SIZE_LABEL} />

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
    </div>
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
      // An item only enters `uploaded` when the API returned its photo.
      const at = coordinateLabel(item.photo!);
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

function ReviewStep({ items, counts, onBack, onDone }: {
  items: readonly Item[];
  counts: BatchTally;
  onBack: () => void;
  onDone: () => void;
}) {
  const uploaded = useMemo(() => items.flatMap((item) => item.photo === undefined ? [] : [item.photo]), [items]);
  const placed = uploaded.filter((photo) => photo.latitude !== null);
  const unplaced = uploaded.filter((photo) => photo.latitude === null);
  return (
    <>
      <Summary counts={counts} />
      <ReviewMap photos={placed} unplacedCount={unplaced.length} />
      <p className="pv-sm pv-muted">All uploaded photos are saved in this voyage. Photos without location data are included but do not appear on the map.</p>
      <div className="pv-inline-3" style={{ marginTop: "var(--pv-space-5)", justifyContent: "flex-end" }}>
        <Button variant="ghost" onClick={onBack}>Back</Button>
        <Button variant="primary" size="lg" onClick={onDone}>View voyage</Button>
      </div>
    </>
  );
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
            // `layout` only returns photos with both coordinates.
            label={`A photo at ${coordinateLabel(photo)!}`}
          />
        ))}
        <Attribution />
      </div>

      <p className="pv-xs pv-muted" style={{ marginTop: "var(--pv-space-2)" }}>
        Check this before publishing. A camera holding an old GPS fix can place a
        photo hundreds of kilometers from where it was taken.
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
