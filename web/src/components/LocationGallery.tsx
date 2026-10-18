import { useModalDialog } from "./useModalDialog";
import { useState } from "react";
import type { VoyagePhoto } from "../api/types";
import { Button } from "./Button";
import { ArrowLeftIcon, ArrowRightIcon, CloseIcon } from "./icons";

/** Native dialog owns modal focus, background inertness and Escape dismissal. */
export function LocationGallery({
  photos,
  onClose,
}: {
  photos: readonly VoyagePhoto[];
  onClose: () => void;
}) {
  const modal = useModalDialog(onClose);
  const [activeId, setActiveId] = useState<string | null>(() => photos[0]?.id ?? null);
  // Keep selection by ID across refreshed ordering; if that photo disappeared,
  // show the first remaining photo (or the empty state).
  const index = Math.max(
    0,
    photos.findIndex((photo) => photo.id === activeId),
  );
  const photo = photos[index];

  return (
    <dialog
      {...modal}
      className="pv-location-gallery"
      aria-label="Photos at this location"
      closedby="any"
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft" && index > 0) {
          event.preventDefault();
          setActiveId(photos[index - 1].id);
        }
        if (event.key === "ArrowRight" && index < photos.length - 1) {
          event.preventDefault();
          setActiveId(photos[index + 1].id);
        }
      }}
    >
      <form method="dialog" className="pv-location-gallery-close">
        <Button type="submit" variant="ghost" size="lg" iconOnly aria-label="Close gallery">
          <CloseIcon size={20} />
        </Button>
      </form>
      <div className="pv-location-gallery-body">
        <Button
          variant="ghost"
          size="lg"
          iconOnly
          aria-label="Previous photo"
          disabled={index === 0}
          onClick={() => setActiveId(photos[index - 1].id)}
        >
          <ArrowLeftIcon size={24} />
        </Button>
        <div className="pv-location-gallery-stage">
          {photo === undefined ? (
            <p className="pv-location-gallery-placeholder pv-sm pv-muted" role="status">
              Photos at this location are no longer available.
            </p>
          ) : (
            <figure key={photo.id}>
              {photo.variant_status === "ready" ? (
                <img
                  src={photo.display_url}
                  alt={`Photograph ${photo.position}`}
                  width={photo.width}
                  height={photo.height}
                  decoding="async"
                />
              ) : (
                <p className="pv-location-gallery-placeholder pv-sm pv-muted" role="status">
                  {photo.variant_status === "pending"
                    ? "Processing photo…"
                    : "This photo could not be processed."}
                </p>
              )}
            </figure>
          )}
        </div>
        <Button
          variant="ghost"
          size="lg"
          iconOnly
          aria-label="Next photo"
          disabled={index >= photos.length - 1}
          onClick={() => setActiveId(photos[index + 1].id)}
        >
          <ArrowRightIcon size={24} />
        </Button>
      </div>
      <footer className="pv-location-gallery-footer">
        <p className="pv-sm pv-muted" aria-live="polite" style={{ margin: 0 }}>
          {photo === undefined ? "0 photos" : `${index + 1} of ${photos.length}`}
        </p>
      </footer>
    </dialog>
  );
}
