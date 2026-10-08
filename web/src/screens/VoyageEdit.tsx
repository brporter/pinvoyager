/**
 * Voyage edit — implements `design/screens/voyage-edit.html`.
 *
 * Metadata on the left, publishing decisions on the right; one column at 390px
 * with the same DOM order, so there is one set of controls at every width
 * rather than two with one hidden.
 *
 * ## Why the precision control sits directly above Publish
 *
 * Adjacency is the argument. Every voyage is world-readable once published
 * (decision D6), so the last thing the user sees before making that decision is
 * the precision it will be published at — and the EXIF-stripping guarantee is
 * stated there, beside the choice, because without stripping the control is
 * decorative: a served JPEG carrying its original GPS tags hands out the exact
 * coordinates to anyone who downloads it, whatever the setting says. That
 * guarantee comes with `GranularitySelector` and there is no prop to turn it
 * off.
 *
 * ## Where validation goes
 *
 * Publishing requires a title, and the server answers a titleless publish with
 * `validation_failed` and an `errors` array of `{field, message}`. Those are
 * mapped onto the inputs they name, so "A title is required before publishing"
 * appears under the title field rather than in a banner at the top of a form
 * whose offending input may be off screen. Anything without a field — a 500, a
 * conflict — falls back to a toast beside the button that caused it, since
 * there is no input to attach it to.
 */

import { useState } from "react";
import type { CSSProperties, FormEvent, ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { isApiError } from "../api/client";
import { useDeleteVoyage, useUpdateVoyage, useVoyage } from "../api/hooks";
import type { Granularity, Voyage, VoyagePhoto } from "../api/types";
import {
  Badge,
  Button,
  EmptyState,
  GranularitySelector,
  ImageIcon,
  Photo,
  PhotoGrid,
  Skeleton,
  TextArea,
  TextField,
  Toast,
  WarningTriangleIcon,
} from "../components";
import { useIsDesktop } from "../shell/useIsDesktop";

export function VoyageEdit() {
  const { id } = useParams<{ id: string }>();
  const query = useVoyage(id);

  if (id === undefined || (query.isError && isApiError(query.error) && query.error.status === 404)) {
    return (
      <Screen>
        <EmptyState
          icon={<ImageIcon size={40} />}
          title="This voyage is not here"
          action={
            <Link className="pv-btn pv-btn--secondary" to="/">
              Discover voyages
            </Link>
          }
        >
          It may already have been deleted.
        </EmptyState>
      </Screen>
    );
  }

  if (query.isPending) {
    return (
      <Screen>
        <div aria-busy="true" className="pv-stack-6">
          <span className="pv-sr">Loading this voyage…</span>
          <Skeleton width="40%" height={28} />
          <Skeleton width="100%" height={44} />
          <Skeleton width="100%" height={96} />
        </div>
      </Screen>
    );
  }

  if (query.isError) {
    const requestId = isApiError(query.error) ? query.error.requestId : null;
    return (
      <Screen>
        <EmptyState
          tone="danger"
          icon={<WarningTriangleIcon size={40} />}
          title="This voyage could not be loaded"
          action={
            <Button
              variant="secondary"
              style={TOUCH}
              onClick={() => void query.refetch()}
            >
              Try again
            </Button>
          }
        >
          Nothing has been changed.
          {requestId !== null && <> Quote reference {requestId} if you report it.</>}
        </EmptyState>
      </Screen>
    );
  }

  // Keyed so that navigating between two voyages re-seeds the form from the
  // one now on screen instead of leaving the previous voyage's text in the
  // inputs — which would otherwise be saved onto the wrong voyage.
  return <EditForm key={query.data.id} voyage={query.data} />;
}

/* ==========================================================================
   The form
   ========================================================================== */

interface FormState {
  title: string;
  description: string;
  granularity: Granularity;
}

function EditForm({ voyage }: { voyage: Voyage }) {
  const navigate = useNavigate();
  const desktop = useIsDesktop();

  const update = useUpdateVoyage(voyage.id);
  const remove = useDeleteVoyage();

  const [form, setForm] = useState<FormState>({
    title: voyage.title,
    description: voyage.description ?? "",
    granularity: voyage.granularity,
  });
  const [saved, setSaved] = useState<"draft" | "published" | "unpublished" | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const failure = update.error;
  const titleError = isApiError(failure) ? failure.fieldError("title") : undefined;
  // A failure the server did not attribute to an input. There is nothing to
  // put it beside, so it becomes a toast rather than being dropped.
  const generalError =
    failure !== null && titleError === undefined ? messageOf(failure) : null;

  const patch = () => ({
    title: form.title,
    // Three states on the wire: absent leaves it alone, null clears it, a
    // string sets it. An emptied textarea means "clear".
    description: form.description.trim() === "" ? null : form.description,
    granularity: form.granularity,
  });

  const save = (published?: boolean) => {
    setSaved(null);
    update.mutate(published === undefined ? patch() : { ...patch(), published }, {
      onSuccess: () => {
        setSaved(published === undefined ? "draft" : published ? "published" : "unpublished");
      },
    });
  };

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    // Enter in a text input submits the form, and the safe meaning of that on
    // this screen is "save the draft", never "publish".
    event.preventDefault();
    save();
  };

  const unplacedCount = voyage.photos.filter((photo) => !photo.placed).length;

  return (
    <Screen>
      <form onSubmit={onSubmit} noValidate>
        <header
          className="pv-inline-2"
          style={{ justifyContent: "space-between", gap: "var(--pv-space-4)", marginBottom: "var(--pv-space-6)" }}
        >
          <div className="pv-inline-2">
            <h1 className="pv-h2">Edit voyage</h1>
            {/* Persistent, in the header at both widths, rather than inferred
                from the position of a toggle further down the page. */}
            <Badge tone={voyage.published ? "public" : "draft"}>
              {voyage.published ? "Public" : "Draft"}
            </Badge>
          </div>

          <div className="pv-inline-2">
            <Button
              variant="ghost"
              size="lg"
              onClick={() => void navigate(`/voyages/${voyage.id}`)}
            >
              Discard
            </Button>
            <Button
              variant="secondary"
              size="lg"
              loading={update.isPending}
              onClick={() => save()}
            >
              Save draft
            </Button>
            {!voyage.published && (
              <Button variant="primary" size="lg" loading={update.isPending} onClick={() => save(true)}>
                Publish voyage
              </Button>
            )}
          </div>
        </header>

        <div
          style={{
            display: "grid",
            gridTemplateColumns: desktop ? "1fr 360px" : "1fr",
            gap: "var(--pv-space-8)",
          }}
        >
          <div className="pv-stack-6">
            <TextField
              label="Title"
              required
              value={form.title}
              error={titleError}
              onChange={(event) => setForm({ ...form, title: event.target.value })}
            />

            <TextArea
              label="Description"
              rows={3}
              value={form.description}
              help="Optional. Shown under the title on the voyage page."
              onChange={(event) => setForm({ ...form, description: event.target.value })}
            />

            <section>
              <div
                className="pv-inline-2"
                style={{ justifyContent: "space-between", marginBottom: "var(--pv-space-3)" }}
              >
                <h2 className="pv-field-label" style={{ margin: 0 }}>
                  Photos · {voyage.photo_count}
                </h2>
                <Link className="pv-btn pv-btn--secondary pv-btn--sm" to={`/upload?voyage=${voyage.id}`} style={TOUCH}>
                  Add photos
                </Link>
              </div>

              {voyage.photos.length === 0 ? (
                <EmptyState
                  icon={<ImageIcon size={40} />}
                  title="No photographs yet"
                  action={
                    <Link className="pv-btn pv-btn--secondary" to={`/upload?voyage=${voyage.id}`} style={TOUCH}>
                      Upload photos
                    </Link>
                  }
                >
                  A voyage with nothing in it has nothing to put on the map.
                </EmptyState>
              ) : (
                <>
                  <PhotoGrid label="Photos in this voyage">
                    {voyage.photos.map((photo) => (
                      <Photo
                        key={photo.id}
                        src={photo.thumbnail_url}
                        alt={photoAlt(photo)}
                        ratio="1:1"
                        unplaced={!photo.placed}
                        loading={photo.variant_status === "pending"}
                        badge={
                          photo.placed ? undefined : <Badge tone="unplaced">No location</Badge>
                        }
                      />
                    ))}
                  </PhotoGrid>

                  {unplacedCount > 0 && (
                    // The discrepancy between "48 photos" and the number of
                    // pins gets its explanation on the same screen.
                    <p className="pv-xs pv-muted" style={{ marginTop: "var(--pv-space-3)" }}>
                      Dashed outlines mark the {unplacedCount}{" "}
                      {unplacedCount === 1 ? "photo" : "photos"} with no location data. They
                      belong to the voyage but will not appear on the map.
                    </p>
                  )}
                </>
              )}
            </section>
          </div>

          <aside className="pv-stack-6">
            <div className="pv-card" style={CARD}>
              <h2 className="pv-h3" style={CARD_TITLE}>
                Location precision
              </h2>
              <GranularitySelector
                value={form.granularity}
                onChange={(granularity) => setForm({ ...form, granularity })}
                labels={{ [voyage.granularity]: voyage.granularity_label }}
                disabled={voyage.placed_count === 0}
                disabledReason="None of the photos in this voyage have location data, so there is nothing to coarsen yet."
              />
            </div>

            {/* Directly below the precision, at both widths. */}
            <div className="pv-card" style={CARD}>
              <h2 className="pv-h3" style={{ ...CARD_TITLE, marginBottom: "var(--pv-space-2)" }}>
                {voyage.published ? "Published" : "Publish"}
              </h2>
              <p className="pv-xs pv-muted" style={{ marginBottom: "var(--pv-space-4)" }}>
                {voyage.published
                  ? "This voyage is visible to everyone, including people who are not signed in."
                  : "Publishing makes this voyage visible to everyone, including people who are not signed in. You can unpublish it later."}
              </p>

              {generalError !== null && (
                <div style={{ marginBottom: "var(--pv-space-3)" }}>
                  <Toast tone="error" title="That did not save">
                    {generalError}
                  </Toast>
                </div>
              )}

              {saved !== null && (
                <div style={{ marginBottom: "var(--pv-space-3)" }}>
                  <Toast tone="success" title={SAVED_TITLE[saved]} onDismiss={() => setSaved(null)}>
                    {saved === "published"
                      ? `Locations are shown at ${voyage.granularity_label}.`
                      : "Your changes are saved."}
                  </Toast>
                </div>
              )}

              {voyage.published ? (
                <Button
                  variant="secondary"
                  style={{ ...TOUCH, width: "100%" }}
                  loading={update.isPending}
                  onClick={() => save(false)}
                >
                  Unpublish voyage
                </Button>
              ) : null}
            </div>

            <div className="pv-card" style={{ ...CARD, borderColor: "var(--pv-danger-bg)" }}>
              <h2 className="pv-h3" style={{ ...CARD_TITLE, marginBottom: "var(--pv-space-2)" }}>
                Delete
              </h2>
              {/* Saying where the photographs go is the point of this
                  sentence: the fear that deleting a voyage destroys the
                  pictures is why dead voyages are left lying around. */}
              <p className="pv-xs pv-muted" style={{ marginBottom: "var(--pv-space-3)" }}>
                Deleting removes the voyage and all of its photos.
              </p>

              {confirmingDelete ? (
                <div className="pv-inline-2">
                  <Button
                    variant="danger"
                    style={TOUCH}
                    loading={remove.isPending}
                    onClick={() =>
                      remove.mutate(voyage.id, { onSuccess: () => void navigate("/") })
                    }
                  >
                    Delete permanently
                  </Button>
                  <Button variant="ghost" style={TOUCH} onClick={() => setConfirmingDelete(false)}>
                    Keep it
                  </Button>
                </div>
              ) : (
                <Button
                  variant="danger"
                  style={{ ...TOUCH, width: "100%" }}
                  onClick={() => setConfirmingDelete(true)}
                >
                  Delete voyage
                </Button>
              )}

              {remove.error !== null && (
                <div style={{ marginTop: "var(--pv-space-3)" }}>
                  <Toast tone="error" title="That did not delete">
                    {messageOf(remove.error)}
                  </Toast>
                </div>
              )}
            </div>
          </aside>
        </div>
      </form>
    </Screen>
  );
}

/* ==========================================================================
   Chrome
   ========================================================================== */

/** The scroll container the design draws: padded, and capped at 900px so the
 *  two columns stay a readable measure on a wide display. */
function Screen({ children }: { children: ReactNode }) {
  return (
    <div style={{ padding: "var(--pv-space-6) var(--pv-space-4)" }}>
      <div style={{ maxWidth: 900, margin: "0 auto" }}>{children}</div>
    </div>
  );
}

const CARD: CSSProperties = { padding: "var(--pv-space-4)" };

const CARD_TITLE: CSSProperties = {
  fontSize: "var(--pv-text-base)",
  marginBottom: "var(--pv-space-3)",
};

/** `.pv-btn` is 40px tall and `.pv-btn--sm` is 32px; both are under the 44px
 *  floor this screen has to meet on a phone. Raised per call site rather than
 *  in the stylesheet, which this screen does not own. */
const TOUCH: CSSProperties = { minHeight: "var(--pv-touch-min)" };

const SAVED_TITLE: Record<"draft" | "published" | "unpublished", string> = {
  draft: "Draft saved",
  published: "Voyage published",
  unpublished: "Voyage unpublished",
};

function messageOf(error: Error): string {
  return error.message === "" ? "Something went wrong at our end." : error.message;
}

function photoAlt(photo: VoyagePhoto): string {
  return photo.placed
    ? `Photograph ${photo.position} in this voyage`
    : `Photograph ${photo.position} in this voyage, no location`;
}
