import { useEffect, useId, useRef, useState } from "react";
import type { VoyagePhoto } from "../api/types";
import { Button } from "./Button";
import { ArrowLeftIcon, ArrowRightIcon, CloseIcon } from "./icons";

/** Native dialog owns modal focus, background inertness and Escape dismissal. */
export function LocationGallery({ photos, precisionLabel, onClose }: {
  photos: readonly VoyagePhoto[];
  precisionLabel: string;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [activeId, setActiveId] = useState<string | null>(() => photos[0]?.id ?? null);
  const index = Math.max(0, photos.findIndex(photo => photo.id === activeId));
  const photo = photos[index];
  const outsidePress = useRef(false);
  const closingInternally = useRef(false);
  useEffect(() => {
    const dialog = dialogRef.current!;
    closingInternally.current = false;
    dialog.showModal();
    return () => {
      closingInternally.current = true;
      if (dialog.open) dialog.close();
    };
  }, []);

  function outsideDialog(event: {
    target: EventTarget; currentTarget: HTMLDialogElement; clientX: number; clientY: number;
  }): boolean {
    if (event.target !== event.currentTarget) return false;
    const bounds = event.currentTarget.getBoundingClientRect();
    return event.clientX < bounds.left || event.clientX > bounds.right
      || event.clientY < bounds.top || event.clientY > bounds.bottom;
  }

  return (
    <dialog ref={dialogRef} className="pv-location-gallery" aria-labelledby={titleId} closedby="any"
      onPointerDown={event => { outsidePress.current = event.button === 0 && outsideDialog(event); }}
      onClick={event => {
        // Also support browsers without native closedby. Both ends must be
        // outside, so dragging a photo out of the frame does not dismiss it.
        if (outsidePress.current && event.button === 0 && outsideDialog(event)) event.currentTarget.close();
        outsidePress.current = false;
      }}
      onKeyDown={event => {
        if (event.key === "ArrowLeft" && index > 0) {
          event.preventDefault();
          setActiveId(photos[index - 1].id);
        }
        if (event.key === "ArrowRight" && index < photos.length - 1) {
          event.preventDefault();
          setActiveId(photos[index + 1].id);
        }
      }}
      onClose={event => {
        // Ignore StrictMode cleanup, including its queued close after reopen.
        if (closingInternally.current || event.currentTarget.open) return;
        onClose();
      }}>
      <header className="pv-location-gallery-header">
        <h2 id={titleId} className="pv-h3" style={{ margin: 0 }}>Photos at this location</h2>
        <p className="pv-sm pv-muted" style={{ margin: 0 }}>
          Photos grouped at this map zoom · {precisionLabel}
        </p>
        <form method="dialog" className="pv-location-gallery-close">
          <Button type="submit" variant="ghost" size="lg" iconOnly aria-label="Close gallery">
            <CloseIcon size={20} />
          </Button>
        </form>
      </header>
      <div className="pv-location-gallery-body">
        <Button variant="ghost" size="lg" iconOnly aria-label="Previous photo" disabled={index === 0}
          onClick={() => setActiveId(photos[index - 1].id)}>
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
                <img src={photo.display_url} alt={`Photograph ${photo.position}`} width={photo.width}
                  height={photo.height} decoding="async" />
              ) : (
                <p className="pv-location-gallery-placeholder pv-sm pv-muted" role="status">
                  {photo.variant_status === "pending" ? "Processing photo…" : "This photo could not be processed."}
                </p>
              )}
              <figcaption className="pv-sm pv-muted">Photograph {photo.position}</figcaption>
            </figure>
          )}
        </div>
        <Button variant="ghost" size="lg" iconOnly aria-label="Next photo" disabled={index >= photos.length - 1}
          onClick={() => setActiveId(photos[index + 1].id)}>
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
