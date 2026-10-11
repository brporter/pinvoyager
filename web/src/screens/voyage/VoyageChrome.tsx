import { useRef, useState } from "react";
import { Link } from "react-router";

import { useUpdateVoyage } from "../../api/hooks";
import type { Voyage, VoyagePhoto } from "../../api/types";
import { useAuth } from "../../auth/useAuth";
import { Badge, Button, Photo, PhotoGrid, ShareIcon, Toast } from "../../components";
import { VoyageShareDialog } from "../../components/VoyageShareDialog";

import { capturedTimeLabel, countLabel, photoAlt } from "./model";
export function BackIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="m15 18-6-6 6-6" />
    </svg>
  );
}

/* ==========================================================================
   Shared pieces
   ========================================================================== */

export function VoyageHeading({ voyage }: { voyage: Voyage }) {
  const { user } = useAuth();
  const owner = user !== null && user.id === voyage.owner_id;

  return (
    <>
      <div className="pv-inline-2" style={{ marginBottom: "var(--pv-space-2)" }}>
        <Badge tone={voyage.published ? "public" : "draft"}>
          {voyage.published ? "Public" : "Draft"}
        </Badge>
        <span className="pv-spacer" />
        {owner && (
          <Link to={`/voyages/${voyage.id}/edit`} className="pv-btn pv-btn--secondary pv-btn--sm">
            Edit
          </Link>
        )}
        <ShareVoyage voyage={voyage} />
      </div>

      <h1 className="pv-h2">{voyage.title}</h1>
      {owner && !voyage.published && <PublishVoyage voyageId={voyage.id} />}

      {voyage.description !== null && voyage.description !== "" && (
        <p className="pv-sm pv-muted" style={{ margin: "var(--pv-space-2) 0 var(--pv-space-3)" }}>
          {voyage.description}
        </p>
      )}

      <div className="pv-inline-2 pv-xs pv-subtle">
        <span>{countLabel(voyage.photo_count, "photo")}</span>
        <span aria-hidden="true">·</span>
        <span>{countLabel(voyage.place_count, "place")}</span>
      </div>
    </>
  );
}

export function ShareVoyage({ voyage }: { voyage: Voyage }) {
  const { user } = useAuth();
  const [sharing, setSharing] = useState(false);
  const trigger = useRef<HTMLButtonElement | null>(null);
  return (
    <>
      <Button
        variant="ghost"
        size="lg"
        iconOnly
        aria-label="Share voyage"
        onClick={(event) => {
          trigger.current = event.currentTarget;
          setSharing(true);
        }}
      >
        <ShareIcon size={18} />
      </Button>
      {sharing && (
        <VoyageShareDialog
          voyage={voyage}
          canPublish={user !== null && user.id === voyage.owner_id}
          onClose={() => {
            setSharing(false);
            trigger.current!.focus();
          }}
        />
      )}
    </>
  );
}

export function PublishVoyage({ voyageId }: { voyageId: string }) {
  const update = useUpdateVoyage(voyageId);
  return (
    <div className="pv-stack-3" style={{ marginBlock: "var(--pv-space-3)" }}>
      <Button
        variant="primary"
        size="lg"
        loading={update.isPending}
        style={{ alignSelf: "flex-start" }}
        onClick={() => update.mutate({ published: true })}
      >
        Publish voyage
      </Button>
      {update.error !== null && (
        <Toast tone="error" title="The voyage could not be published">
          {update.error.message}
        </Toast>
      )}
    </div>
  );
}

interface SelectablePhotoProps {
  photo: VoyagePhoto;
  selected: boolean;
  onSelect: (photoId: string) => void;
  registerPhoto: (photoId: string, node: HTMLElement | null) => void;
}

/**
 * A photograph in the rail or the strip.
 *
 * Wrapped in a `div` that holds the ref rather than reaching into `Photo`:
 * scrolling wants an element, `Photo` does not forward one, and the wrapper is
 * the same box either way.
 */
export function SelectablePhoto({
  photo,
  selected,
  onSelect,
  registerPhoto,
}: SelectablePhotoProps) {
  return (
    <div ref={(node) => registerPhoto(photo.id, node)}>
      <Photo
        src={photo.thumbnail_url}
        alt={photoAlt(photo)}
        ratio="1:1"
        selected={selected}
        unplaced={!photo.placed}
        loading={photo.variant_status === "pending"}
        onSelect={() => onSelect(photo.id)}
        label={photoAlt(photo)}
        caption={selected ? <span className="pv-xs">{capturedTimeLabel(photo)}</span> : undefined}
      />
    </div>
  );
}

interface UnplacedSectionProps {
  photos: readonly VoyagePhoto[];
  selectedPhotoId: string | null;
  onSelect: (photoId: string) => void;
  registerPhoto: (photoId: string, node: HTMLElement | null) => void;
}

/**
 * Unplaced photographs, at the bottom of the rail.
 *
 * They are given a home rather than being dropped. A user who uploaded 54
 * photos and counts 48 on the map needs to be told where the other six went,
 * and "nowhere, silently" is the answer that makes the map look broken.
 */
export function UnplacedSection({
  photos,
  selectedPhotoId,
  onSelect,
  registerPhoto,
}: UnplacedSectionProps) {
  const [open, setOpen] = useState(false);

  if (photos.length === 0) return null;

  return (
    <section style={{ marginTop: "var(--pv-space-6)" }}>
      <div
        className="pv-card"
        style={{
          padding: "var(--pv-space-3)",
          background: "var(--pv-warning-bg)",
          borderColor: "transparent",
        }}
      >
        <div className="pv-inline-2" style={{ justifyContent: "space-between" }}>
          <div>
            <div className="pv-sm" style={{ fontWeight: "var(--pv-weight-semibold)" }}>
              {photos.length === 1
                ? "1 photo has no location"
                : `${photos.length} photos have no location`}
            </div>
            <div className="pv-xs pv-muted">They are part of this voyage but not on the map.</div>
          </div>
          <Button
            variant="secondary"
            size="sm"
            aria-expanded={open}
            style={{ minHeight: "var(--pv-touch-min)" }}
            onClick={() => setOpen((current) => !current)}
          >
            {open ? "Hide" : "View"}
          </Button>
        </div>
      </div>

      {open && (
        <div style={{ marginTop: "var(--pv-space-3)" }}>
          <PhotoGrid label="Photos with no location">
            {photos.map((photo) => (
              <SelectablePhoto
                key={photo.id}
                photo={photo}
                selected={photo.id === selectedPhotoId}
                onSelect={onSelect}
                registerPhoto={registerPhoto}
              />
            ))}
          </PhotoGrid>
        </div>
      )}
    </section>
  );
}

/* ==========================================================================
   Loading, empty and error
   ========================================================================== */
