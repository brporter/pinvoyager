/** Draft creation and per-file upload/review. The batch hook owns cancellation and retry identity. */

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";

import { keys, useCreateVoyage } from "../api/hooks";
import type { Granularity, Photo } from "../api/types";
import {
  Badge,
  Button,
  Dropzone,
  EmptyState,
  GranularitySelector,
  ImageIcon,
  ShieldCheckIcon,
  TextField,
  TextArea,
  Toast,
  UploadRow,
} from "../components";

import { LazyUploadReviewMap } from "../map";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

import {
  MAX_SIZE_LABEL,
  tally,
  coordinateLabel,
  plural,
  type Item,
  type BatchTally,
} from "./upload/model";
import { useUploadBatch } from "./upload/useUploadBatch";

type Step = "details" | "choose" | "process" | "review";

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

export function Upload() {
  const [search, setSearch] = useSearchParams();
  const selected = search.get("voyage");
  const [flow, setFlow] = useState({
    destination: selected,
    key: 0,
    created: undefined as string | undefined,
  });
  if (flow.destination !== selected) {
    setFlow({
      destination: selected,
      key: flow.created === selected ? flow.key : flow.key + 1,
      created: undefined,
    });
    return null;
  }
  return (
    <UploadFlow
      key={flow.key}
      initialVoyageId={selected}
      onCreated={(id) => {
        // Recording our own creation keeps this batch and its trace context alive.
        setFlow((current) => ({ ...current, created: id }));
        setSearch({ voyage: id }, { replace: true });
      }}
    />
  );
}

/** Navigation to another destination unmounts this flow and cancels its batch. */
function UploadFlow({
  initialVoyageId,
  onCreated,
}: {
  initialVoyageId: string | null;
  onCreated: (id: string) => void;
}) {
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const navigate = useNavigate();
  const [voyageId, setVoyageId] = useState(initialVoyageId);
  const destination = useRef(voyageId);
  const traceContext = useRef<import("../api/client").TraceContext>({});
  const [title, setTitle] = useState("");
  const [nameError, setNameError] = useState<string>();
  const [description, setDescription] = useState("");
  const [descriptionError, setDescriptionError] = useState<string>();
  const [precision, setPrecision] = useState<Granularity>("km1");
  const queryClient = useQueryClient();
  const createMutation = useCreateVoyage({ traceContext: traceContext.current });

  const [step, setStep] = useState<Step>(voyageId === null ? "details" : "choose");
  const batch = useUploadBatch(destination, traceContext.current);
  const { items } = batch;

  const create = useMutation({
    mutationFn: async (granularity: Granularity) => {
      const voyage = await createMutation.mutateAsync({
        title: title.trim(),
        description: description.trim(),
        granularity,
      });
      if (!active.current) return;
      destination.current = voyage.id;
      setVoyageId(voyage.id);
      onCreated(voyage.id);
      setStep("choose");
    },
    retry: false,
  });

  const onFiles = (files: File[]) => {
    batch.add(files);
    setStep("process");
  };

  const counts = tally(items);

  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-6)" }}>
      <div style={{ maxWidth: 720, margin: "0 auto" }}>
        <h1 className="pv-h2" style={{ marginBottom: "var(--pv-space-5)" }}>
          {step === "details"
            ? "Create voyage"
            : step === "choose"
              ? "Add photos"
              : headingFor(step, counts)}
        </h1>

        <Steps current={step} />

        {step === "details" && (
          <form
            className="pv-stack-6"
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              const missingName = title.trim() === "";
              const missingDescription = description.trim() === "";
              setNameError(missingName ? "Enter a voyage name." : undefined);
              setDescriptionError(missingDescription ? "Enter a voyage description." : undefined);
              if (missingName || missingDescription) return;
              create.mutate(precision);
            }}
          >
            <p className="pv-sm pv-muted" style={{ margin: 0 }}>
              You will be able to upload your photos at the next step.
            </p>
            <TextField
              label="Voyage Name"
              required
              value={title}
              error={nameError}
              disabled={create.isPending}
              onChange={(event) => {
                setTitle(event.target.value);
                setNameError(undefined);
              }}
            />
            <TextArea
              label="Voyage Description"
              required
              value={description}
              error={descriptionError}
              rows={4}
              disabled={create.isPending}
              onChange={(event) => {
                setDescription(event.target.value);
                setDescriptionError(undefined);
              }}
            />
            <fieldset disabled={create.isPending} style={{ border: 0, padding: 0, margin: 0 }}>
              <legend className="pv-sm" style={{ marginBottom: "var(--pv-space-3)" }}>
                Location precision (required)
              </legend>
              <GranularitySelector
                value={precision}
                disabled={create.isPending}
                onChange={setPrecision}
              />
            </fieldset>
            {create.isError && (
              <Toast tone="error" title="The voyage could not be created">
                {create.error.message}
              </Toast>
            )}
            <div className="pv-inline-3" style={{ justifyContent: "flex-end" }}>
              <Button
                type="submit"
                variant="primary"
                size="lg"
                loading={create.isPending}
                disabled={create.isPending}
              >
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
            onCancel={batch.cancel}
            onRetry={batch.retry}
            onRemove={batch.remove}
            onReview={() => setStep("review")}
            onStartOver={() => {
              batch.reset();
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

/** Displays the four upload steps and marks the current one. */
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
        const state = position < index ? "done" : position === index ? "current" : "todo";

        return (
          <li
            key={step}
            aria-current={state === "current" ? "step" : undefined}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--pv-space-2)",
              fontSize: "var(--pv-text-sm)",
              color: state === "todo" ? "var(--pv-text-subtle)" : "var(--pv-text)",
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

function ChooseStep({
  onFiles,
  disabled,
}: {
  onFiles: (files: File[]) => void;
  disabled: boolean;
}) {
  return (
    <div>
      <Dropzone
        onFiles={onFiles}
        disabled={disabled}
        formatsLabel="JPEG, PNG, WebP, HEIC"
        maxSizeLabel={MAX_SIZE_LABEL}
      />

      {/* Explain the image metadata and location handling before file selection. */}
      <div className="pv-gran-guarantee" style={{ marginTop: "var(--pv-space-5)" }}>
        <ShieldCheckIcon size={16} />
        <div>
          <strong>Your originals are never published.</strong> PinVoyager reads the location and
          time from each photo, then serves re-encoded copies with all metadata removed.
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
  const processed = counts.total - counts.pending;
  const current = items.find((item) => item.status === "uploading" || item.status === "processing");
  const failures = items.filter((item) => item.status === "error");

  return (
    <>
      <div className="pv-card" style={{ padding: "var(--pv-space-4)" }}>
        <p className="pv-sm" style={{ marginTop: 0 }}>
          {processed} of {counts.total} {plural(counts.total, "file", "files")} processed
        </p>
        <div
          className="pv-progress"
          role="progressbar"
          aria-label="File processing progress"
          aria-valuemin={0}
          aria-valuemax={Math.max(1, counts.total)}
          aria-valuenow={processed}
          aria-valuetext={`${processed} of ${counts.total} ${plural(counts.total, "file", "files")} processed`}
        >
          <div
            className="pv-progress-bar"
            style={{ width: `${counts.total === 0 ? 0 : (processed / counts.total) * 100}%` }}
          />
        </div>
        <div className="pv-upload-current" aria-live="polite">
          {current !== undefined ? (
            <>
              <span className="pv-upload-info">
                <span className="pv-upload-name">{current.file.name}</span>
                <span className="pv-upload-sub">
                  {current.status === "processing" ? " · reading location data" : " · uploading"}
                </span>
              </span>
              <Button
                variant="ghost"
                size="sm"
                aria-label={`Cancel upload of ${current.file.name}`}
                onClick={() => onCancel(current.id)}
              >
                Cancel file
              </Button>
            </>
          ) : (
            <span className="pv-sm pv-muted">File processing complete.</span>
          )}
        </div>
        <p className="pv-xs pv-muted" style={{ marginBottom: 0 }}>
          Photos are saved as they are accepted. Image preparation may continue after upload.
        </p>
      </div>

      {failures.length > 0 && (
        <div
          className="pv-card pv-upload-errors"
          role="region"
          aria-label="Files that were not uploaded"
          tabIndex={0}
        >
          {failures.map((item) => (
            <UploadRow
              key={item.id}
              name={item.file.name}
              status="error"
              error={item.failure.message}
              onRetry={item.failure.retryable ? () => onRetry(item.id) : undefined}
              onRemove={() => onRemove(item.id)}
            />
          ))}
        </div>
      )}

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
              Review {counts.uploaded} {plural(counts.uploaded, "photo", "photos")}
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
          No photos remain in this batch. Any file errors are listed above.
        </EmptyState>
      )}
    </>
  );
}

/** Reports successful uploads, failures, cancellations, and missing locations. */
function Summary({ counts }: { counts: BatchTally }) {
  const notes: string[] = [];
  if (counts.unplaced > 0) {
    notes.push(`${String(counts.unplaced)} ${plural(counts.unplaced, "has", "have")} no location`);
  }
  if (counts.failed > 0) {
    notes.push(`${String(counts.failed)} ${plural(counts.failed, "was", "were")} skipped`);
  }

  return (
    <div className="pv-card" style={{ padding: "var(--pv-space-4)" }}>
      <div style={{ fontWeight: "var(--pv-weight-semibold)" }}>
        {counts.uploaded} of {counts.total} {plural(counts.total, "photo", "photos")} uploaded
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
  onDone: () => void;
}) {
  const uploaded = useMemo(
    () => items.flatMap((item) => (item.photo === undefined ? [] : [item.photo])),
    [items],
  );
  const unplaced = uploaded.filter((photo) => photo.latitude === null || photo.longitude === null);
  return (
    <>
      <Summary counts={counts} />
      <ReviewMap photos={uploaded} unplacedCount={unplaced.length} />
      <p className="pv-sm pv-muted">
        All uploaded photos are saved in this voyage. Photos without location data are included but
        do not appear on the map.
      </p>
      <div
        className="pv-inline-3"
        style={{ marginTop: "var(--pv-space-5)", justifyContent: "flex-end" }}
      >
        <Button variant="ghost" onClick={onBack}>
          Back
        </Button>
        <Button variant="primary" size="lg" onClick={onDone}>
          View voyage
        </Button>
      </div>
    </>
  );
}

/** Review exact uploaded coordinates on the shared interactive basemap. */
function ReviewMap({ photos, unplacedCount }: { photos: readonly Photo[]; unplacedCount: number }) {
  const pins = useMemo(
    () =>
      photos.flatMap((photo) =>
        photo.latitude === null || photo.longitude === null
          ? []
          : [
              {
                id: photo.id,
                coordinate: { latitude: photo.latitude, longitude: photo.longitude },
                label: `A photo at ${coordinateLabel(photo)!}`,
              },
            ],
      ),
    [photos],
  );

  return (
    <div style={{ marginTop: "var(--pv-space-4)" }}>
      <LazyUploadReviewMap
        pins={pins}
        label={`Where these photos landed — ${String(pins.length)} on the map`}
        style={{
          height: "clamp(180px, 34vw, 300px)",
          borderRadius: "var(--pv-radius-lg)",
          position: "relative",
        }}
      />

      <p className="pv-xs pv-muted" style={{ marginTop: "var(--pv-space-2)" }}>
        Check this before publishing. A camera holding an old GPS fix can place a photo hundreds of
        kilometers from where it was taken.
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
