import type { CSSProperties } from "react";
import type { Coordinate, Voyage, VoyagePhoto } from "../../api/types";
import type { MapSelection, VoyagePin } from "../../map";
export type Selection = MapSelection;
export const RAIL_WIDTH = 420;
export function fillHeight(desktop: boolean): CSSProperties {
  return {
    height: "100%",
    minHeight: desktop
      ? "calc(100dvh - var(--pv-header-h))"
      : "calc(100dvh - var(--pv-header-h) - var(--pv-tabbar-h))",
  };
}

export interface LayoutProps {
  voyage: Voyage;
  /** The whole selection, not just the id: the map needs to know which half of
   *  the screen asked, so that clicking a pin does not also re-center on it. */
  selection: Selection | null;
  onSelectPin: (photoId: string) => void;
  onOpenGallery: (photoIds: string[]) => void;
  onSelectPhoto: (photoId: string) => void;
  onClearSelection: () => void;
  registerPhoto: (photoId: string, node: HTMLElement | null) => void;
}

/**
 * The pins for a voyage, built from the photographs it already carries.
 *
 * Photographs with no coordinate are dropped here and picked up by
 * {@link UnplacedSection} instead — the map genuinely cannot draw them, and
 * they are accounted for in the rail rather than silently lost.
 *
 * The label is {@link photoAlt}, the same string the photograph carries in the
 * rail. That is deliberate: it is the only accessible name a pin has, and
 * sharing it is what lets a screen-reader user tell which pin belongs to which
 * photograph.
 */
export function pinsFor(voyage: Voyage): VoyagePin[] {
  return voyage.photos
    .filter((photo): photo is VoyagePhoto & { coordinate: Coordinate } => photo.coordinate !== null)
    .map((photo) => ({
      id: photo.id,
      coordinate: photo.coordinate,
      label: photoAlt(photo),
    }));
}

/* ==========================================================================
   Desktop — map left, rail right
   ========================================================================== */

export const CHIP: CSSProperties = {
  background: "var(--pv-bg)",
  borderRadius: "var(--pv-radius-full)",
  boxShadow: "var(--pv-shadow-2)",
  display: "flex",
  alignItems: "center",
  gap: "var(--pv-space-2)",
  padding: "var(--pv-space-2) var(--pv-space-3)",
  minHeight: "var(--pv-touch-min)",
};

/** Not in `components/icons` — the icon set there has no chevron. */
const DAY_FORMAT = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long" });
const TIME_FORMAT = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

export function placedPhotos(voyage: Voyage): VoyagePhoto[] {
  return voyage.photos.filter((photo) => photo.placed);
}

export function parseCaptured(photo: VoyagePhoto): Date | null {
  if (photo.captured_at === null) return null;
  const date = new Date(photo.captured_at);
  return Number.isNaN(date.getTime()) ? null : date;
}

interface DateGroup {
  key: string;
  label: string;
  photos: VoyagePhoto[];
}

/**
 * Groups by capture date, in the order the photographs arrive.
 *
 * Chronology is the only ordering a travelogue needs no explanation for, and
 * the API already returns photographs in voyage position order. Photographs
 * with no capture time land in one trailing group rather than being hidden —
 * the same principle as unplaced photographs.
 */
export function groupByCaptureDate(photos: readonly VoyagePhoto[]): DateGroup[] {
  const groups: DateGroup[] = [];
  const byKey = new Map<string, DateGroup>();

  for (const photo of photos) {
    const captured = parseCaptured(photo);
    const key = captured === null ? "undated" : captured.toDateString();
    const label = captured === null ? "Date unknown" : DAY_FORMAT.format(captured);

    const existing = byKey.get(key);
    if (existing === undefined) {
      const group: DateGroup = { key, label, photos: [photo] };
      byKey.set(key, group);
      groups.push(group);
    } else {
      existing.photos.push(photo);
    }
  }

  // "Date unknown" belongs at the end, after everything that can be ordered.
  return [
    ...groups.filter((g) => g.key !== "undated"),
    ...groups.filter((g) => g.key === "undated"),
  ];
}

export function capturedTimeLabel(photo: VoyagePhoto): string | null {
  const captured = parseCaptured(photo);
  return captured === null ? null : TIME_FORMAT.format(captured);
}

export function capturedAtLabel(photo: VoyagePhoto): string | null {
  const captured = parseCaptured(photo);
  return captured === null
    ? null
    : `${DAY_FORMAT.format(captured)}, ${TIME_FORMAT.format(captured)}`;
}

/** The rounded position, as the viewer would read it. Never the original EXIF:
 *  a voyage's coordinates have already been coarsened server-side. */
export function coordinateLabel(coordinate: Coordinate | null): string | null {
  if (coordinate === null) return null;
  return `${coordinate.latitude.toFixed(2)}°, ${coordinate.longitude.toFixed(2)}°`;
}

/**
 * The accessible name of a photograph, which is also its pin's name.
 *
 * A pin is a graphic with no text, so this is the only name it has — "Photo
 * taken 14 February, 09:12" rather than "Pin". The two sides of the selection
 * deliberately share it, so a screen-reader user can tell which pin belongs to
 * which photograph.
 */
export function photoAlt(photo: VoyagePhoto): string {
  const when = capturedAtLabel(photo);
  const base = when === null ? `Photograph ${photo.position}` : `Photograph taken ${when}`;
  return photo.placed ? base : `${base}, no location`;
}

export function countLabel(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}
