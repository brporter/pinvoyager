/**
 * Map primitives — from `design/components/map-primitives.html`.
 *
 * Presentational only. Phase 7 owns MapLibre; what these establish is the
 * marker geometry and the dimensions that map has to fit into, so the layout is
 * genuinely exercised at both breakpoints before a real map arrives.
 *
 * Positions are inline `left`/`top`, exactly as the design specimens set them.
 * That is data — where a marker is — not styling, and a real map will be
 * feeding the same two numbers from a projection.
 */

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
 * Pin.
 *
 * The visible marker is 26px; the button around it is 44px, per
 * `--pv-touch-min`. Dense clusters on a phone are unusable otherwise — the tap
 * lands between two markers and nothing happens, which reads as the app being
 * broken rather than the finger being imprecise. The offset margins in
 * `.pv-pin` are what keep the *visible* tip on the coordinate while the hit
 * area extends past it.
 *
 * Selection changes hue, scale and halo at once. Hue alone would fail for a
 * meaningful number of users, and on a map the marker often sits on an
 * arbitrary background where a single-signal change is easy to miss even with
 * full colour vision.
 *
 * Without `onClick` it renders as a labelled graphic rather than a button. The
 * design file draws every pin as a `<button>`, but a button that does nothing
 * is a dead stop in the tab order — which is exactly what the granularity
 * preview would otherwise put there.
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
  /** Given, the bubble becomes a button — the usual behaviour is to zoom in. */
  onClick?: () => void;
  /** Overrides the accessible name. Defaults to "N photos here". */
  label?: string;
  className?: string;
}

/**
 * Cluster.
 *
 * `--pv-cluster-bg` is darker than `--pv-pin` on purpose: the bubble carries a
 * number, so it needs 4.5:1 against its label, where a bare pin only needs the
 * 3:1 non-text floor against the basemap.
 *
 * Rendered as a `<button>` when it has a handler, because the design gives it
 * `cursor: pointer` and a pointer-only affordance is not reachable by keyboard.
 * `.pv-cluster` overrides the user agent's button background, border, and
 * padding but not its font family, so a clustered count inherits the platform
 * button font until `font-family: inherit` is added to that rule.
 */
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

/**
 * MapArea.
 *
 * A coarse voyage draws an area, not a point. Rendering a sharp pin at the
 * centre of a 10 km rounding makes a precision claim the data cannot support,
 * and it visually undoes the choice the user deliberately made. The circle's
 * radius reflects the actual precision, so what is drawn is what is known.
 */
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

/**
 * Attribution.
 *
 * OpenStreetMap attribution is required by the ODbL licence under which the map
 * data is published. It is not a courtesy: it appears on every map instance,
 * and removing it is a licence violation. There is no prop to hide it.
 *
 * The design file draws the notice as plain text; `.pv-attribution a` exists in
 * the stylesheet, so the copyright link is rendered here — which is what the
 * licence actually asks for.
 */
export function Attribution({ dark = false, children, className }: AttributionProps) {
  return (
    <div className={cx("pv-attribution", dark && "pv-attribution--dark", className)}>
      {children ?? (
        <>
          ©{" "}
          <a
            href="https://www.openstreetmap.org/copyright"
            target="_blank"
            rel="noreferrer"
          >
            OpenStreetMap
          </a>{" "}
          contributors
        </>
      )}
    </div>
  );
}
