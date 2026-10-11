/** Coordinates voyage data, map/list selection, and the gallery. */
import { useCallback, useEffect, useRef, useState } from "react";
import { useParams } from "react-router";

import { useVoyage } from "../api/hooks";
import { LocationGallery } from "../components/LocationGallery";
import { prefersReducedMotion } from "../shell/motion";
import { useIsDesktop } from "../shell/useIsDesktop";

import { DesktopLayout } from "./voyage/DesktopLayout";
import { MobileLayout } from "./voyage/MobileLayout";
import { type LayoutProps, type Selection } from "./voyage/model";
import { ErrorState, LoadingState, NotFound } from "./voyage/VoyageStates";
export function VoyageDetail() {
  const { id } = useParams<{ id: string }>();
  const query = useVoyage(id);
  const desktop = useIsDesktop();

  const [galleryIds, setGalleryIds] = useState<string[] | null>(null);
  const galleryTrigger = useRef<HTMLElement | null>(null);
  const openGallery = useCallback((ids: string[]) => {
    galleryTrigger.current = document.activeElement as HTMLElement;
    setGalleryIds(ids);
  }, []);

  const [selection, setSelection] = useState<Selection | null>(null);

  /** Photo id → the element showing it, in whichever layout is mounted. Used
   *  only to scroll a pin's photograph into view. */
  const photoNodes = useRef(new Map<string, HTMLElement>());

  const registerPhoto = useCallback((photoId: string, node: HTMLElement | null) => {
    if (node === null) {
      photoNodes.current.delete(photoId);
    } else {
      photoNodes.current.set(photoId, node);
    }
  }, []);

  useEffect(() => {
    if (selection === null || selection.source !== "pin") return;

    const node = photoNodes.current.get(selection.photoId);
    // `scrollIntoView` is absent in jsdom and in any non-browser renderer, and
    // a missing scroll is not worth throwing over.
    node?.scrollIntoView?.({
      block: "nearest",
      inline: "nearest",
      behavior: prefersReducedMotion() ? "auto" : "smooth",
    });
  }, [selection]);

  const selectFromPin = useCallback((photoId: string) => {
    setSelection({ photoId, source: "pin" });
  }, []);

  const selectFromPhoto = useCallback((photoId: string) => {
    setSelection((current) => (current?.photoId === photoId ? null : { photoId, source: "photo" }));
  }, []);

  const clearSelection = useCallback(() => setSelection(null), []);

  if (id === undefined) {
    return <NotFound />;
  }

  if (query.isPending) {
    return <LoadingState desktop={desktop} />;
  }

  if (query.isError) {
    return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  }

  const view: LayoutProps = {
    voyage: query.data,
    selection,
    onSelectPin: selectFromPin,
    onOpenGallery: openGallery,
    onSelectPhoto: selectFromPhoto,
    onClearSelection: clearSelection,
    registerPhoto,
  };

  return (
    <>
      {desktop ? <DesktopLayout {...view} /> : <MobileLayout {...view} />}
      {galleryIds !== null && (
        <LocationGallery
          photos={query.data.photos.filter((photo) => galleryIds.includes(photo.id))}
          onClose={() => {
            setGalleryIds(null);
            const trigger = galleryTrigger.current!;
            if (trigger.isConnected) trigger.focus();
            else {
              // Selection can replace a single-pin button; restore its equivalent.
              const label = trigger.getAttribute("aria-label");
              const replacement = Array.from(
                document.querySelectorAll<HTMLElement>("button.pv-pin"),
              ).find((button) => button.getAttribute("aria-label") === label);
              replacement?.focus();
            }
          }}
        />
      )}
    </>
  );
}
