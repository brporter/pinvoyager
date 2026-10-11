/** Photo uses the shared runtime styles and accessible native controls. */

import type { KeyboardEvent, ReactNode } from "react";

import { cx } from "./cx";

export type PhotoRatio = "4:3" | "1:1" | "3:4";

const RATIO_CLASS: Record<PhotoRatio, string | false> = {
  "4:3": false, // the default in `.pv-photo`
  "1:1": "pv-photo--square",
  "3:4": "pv-photo--tall",
};

export interface PhotoProps {
  src?: string | null;
  /** Empty string is correct for a photograph with no caption of its own: it is
   *  content, but nothing useful can be said about it that the page does not
   *  already say. Pass real text when there is any. */
  alt?: string;
  ratio?: PhotoRatio;
  /** Selection, typically driven from a map pin. */
  selected?: boolean;
  /** No GPS EXIF. Draws the dashed warning border. */
  unplaced?: boolean;
  /** Renders the frame as a skeleton instead of an image. */
  loading?: boolean;
  /** Overlaid on a scrim at the bottom of the frame. */
  caption?: ReactNode;
  /** Overlaid at the top — where the design puts the "No location" badge. */
  badge?: ReactNode;
  /** Makes the frame activatable. Requires `label`. */
  onSelect?: () => void;
  /** Accessible name for the activatable frame. */
  label?: string;
  /** One of the eight CSS gradient stand-ins (`pv-img-1` … `pv-img-8`) used
   *  where there is no real photograph, as in the design specimens. */
  gradient?: 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8;
  className?: string;
}

export function Photo({
  src,
  alt = "",
  ratio = "4:3",
  selected = false,
  unplaced = false,
  loading = false,
  caption,
  badge,
  onSelect,
  label,
  gradient,
  className,
}: PhotoProps) {
  const interactive = onSelect != null;

  const frameClass = cx("pv-photo", RATIO_CLASS[ratio], loading && "pv-skeleton", className);

  const activation = interactive
    ? {
        role: "button" as const,
        tabIndex: 0,
        "aria-label": label,
        "aria-pressed": selected,
        onClick: onSelect,
        onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onSelect();
          }
        },
      }
    : {};

  return (
    <div
      {...activation}
      className={frameClass}
      data-selected={selected ? "true" : undefined}
      data-unplaced={unplaced ? "true" : undefined}
    >
      {loading ? null : src != null && src !== "" ? (
        <img className="pv-photo-img" src={src} alt={alt} loading="lazy" decoding="async" />
      ) : gradient != null ? (
        <div className={`pv-photo-img pv-img-${gradient}`} />
      ) : null}

      {badge != null && (
        // The design file positions this caption block at the top inline;
        // `design/components.css` has no `--top` modifier for it yet.
        <div className="pv-photo-caption" style={{ top: "var(--pv-space-2)", bottom: "auto" }}>
          {badge}
        </div>
      )}

      {caption != null && (
        <>
          <div className="pv-photo-scrim" />
          <div className="pv-photo-caption">{caption}</div>
        </>
      )}
    </div>
  );
}
