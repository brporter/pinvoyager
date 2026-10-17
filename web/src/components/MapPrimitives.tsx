/** Map marker, cluster, precision-area, and attribution presentation components. */

import type { CSSProperties, ReactNode } from "react";

import { cx } from "./cx";

/** A percentage or any CSS length; the design specimens use both. */
export type MapCoord = string | number;

function place(left: MapCoord | undefined, top: MapCoord | undefined): CSSProperties {
  return { left, top };
}

/* ==========================================================================
   Pin
   ========================================================================== */

export interface PinProps {
  /** Required. A pin is a graphic with no text, so this is its only accessible
   *  name — "Photo taken 14 February, 09:12" rather than "Pin". */
  label: string;
  left?: MapCoord;
  top?: MapCoord;
  selected?: boolean;
  onClick?: () => void;
  className?: string;
}

/**
 * Draws a 26px pin within a 44px hit area. Selection changes color, scale,
 * and halo. With onClick it is a button; otherwise it is a labeled graphic.
 */
export function Pin({ label, left, top, selected = false, onClick, className }: PinProps) {
  const shared = {
    className: cx("pv-pin", className),
    style: place(left, top),
    "data-selected": selected ? "true" : undefined,
  };

  if (onClick == null) {
    return (
      <span {...shared} role="img" aria-label={label}>
        <span className="pv-pin-shape" />
      </span>
    );
  }

  return (
    <button {...shared} type="button" aria-label={label} aria-pressed={selected} onClick={onClick}>
      <span className="pv-pin-shape" />
    </button>
  );
}

/* ==========================================================================
   Cluster
   ========================================================================== */

export type ClusterSize = "sm" | "md" | "lg";

export interface ClusterProps {
  /** How many photos the bubble stands for. Rendered as its own label. */
  count: number;
  left?: MapCoord;
  top?: MapCoord;
  /** Three tiers by count, as drawn: 38px, 46px, 56px. */
  size?: ClusterSize;
  /** Given, the bubble becomes a button — the usual behavior is to zoom in. */
  onClick?: () => void;
  /** Overrides the accessible name. Defaults to "N photos here". */
  label?: string;
  className?: string;
}

/** Draws a numbered cluster, using a button when an activation handler is supplied. */
export function Cluster({
  count,
  left,
  top,
  size = "sm",
  onClick,
  label,
  className,
}: ClusterProps) {
  const classes = cx("pv-cluster", size !== "sm" && `pv-cluster--${size}`, className);
  const name = label ?? `${count} photos here`;

  if (onClick == null) {
    return (
      <div className={classes} style={place(left, top)} role="img" aria-label={name}>
        {count}
      </div>
    );
  }

  return (
    <button
      type="button"
      className={classes}
      style={place(left, top)}
      aria-label={name}
      onClick={onClick}
    >
      {count}
    </button>
  );
}

/* ==========================================================================
   MapArea
   ========================================================================== */

export interface MapAreaProps {
  left?: MapCoord;
  top?: MapCoord;
  /** Diameter. The design draws 62px for ~1 km, 132px for ~10 km and 230px for
   *  ~50 km at the preview's scale; a real map derives it from
   *  `Coordinate.precision_m` and the current zoom. */
  size: MapCoord;
  /** Names the area, e.g. "Photos within about 10 km". Without it the circle is
   *  hidden from assistive technology, which is right when a nearby caption
   *  already states the precision. */
  label?: string;
  className?: string;
}

/** Draws a circle at the supplied diameter to indicate approximate location precision. */
export function MapArea({ left, top, size, label, className }: MapAreaProps) {
  return (
    <div
      className={cx("pv-map-area", className)}
      style={{ left, top, width: size, height: size }}
      role={label != null ? "img" : undefined}
      aria-label={label}
      aria-hidden={label == null ? true : undefined}
    />
  );
}

/* ==========================================================================
   Attribution
   ========================================================================== */

export interface AttributionProps {
  /** For a dark basemap. Kept separate from the page theme, because a dark map
   *  can appear on a light page. */
  dark?: boolean;
  children?: ReactNode;
  className?: string;
}

/** Displays linked OpenStreetMap attribution, with a dark-basemap modifier. */
export function Attribution({ dark = false, children, className }: AttributionProps) {
  return (
    <div className={cx("pv-attribution", dark && "pv-attribution--dark", className)}>
      {children ?? (
        <>
          ©{" "}
          <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer">
            OpenStreetMap
          </a>{" "}
          contributors
        </>
      )}
    </div>
  );
}
