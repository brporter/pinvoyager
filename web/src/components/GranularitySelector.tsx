/**
 * GranularitySelector — from `design/components/granularity.html`.
 *
 * The one genuinely novel control in this product, and the most consequential.
 * Every voyage is public, so this setting is the only thing standing between an
 * EXIF coordinate and the world.
 *
 * ## Why there is a map preview and not a dropdown
 *
 * A dropdown reading "~10 km" tells someone nothing about what they are
 * actually revealing. The preview does: it shows the region a viewer would be
 * able to infer, at the scale they would see it. The consequence has to be
 * legible *before* publishing, not after — which is the whole reason this
 * control exists instead of a select.
 *
 * ## Why the guarantee text is not optional
 *
 * Without EXIF stripping the entire control is decorative. If a user picks
 * ~50 km but the served JPEG still carries its original GPS tags, anyone who
 * downloads the image reads the exact coordinates straight out of the file. The
 * promise is therefore stated where the choice is made rather than buried in a
 * policy page, and there is no prop to turn it off.
 *
 * The four steps are toggle buttons with `aria-pressed`, exactly as the design
 * draws them, grouped so a screen reader announces what the set is for.
 */

import type { Granularity } from "../api/types";

import { Attribution, MapArea, Pin } from "./MapPrimitives";
import { ShieldCheckIcon } from "./icons";

export const GRANULARITIES: readonly Granularity[] = ["exact", "km1", "km10", "km50"];

/** Fallback labels. Prefer the server's `granularity_label` where you have a
 *  voyage to hand — it is rendered rather than derived so the several places it
 *  appears cannot disagree. */
export const GRANULARITY_LABEL: Record<Granularity, string> = {
  exact: "Exact",
  km1: "~1 km",
  km10: "~10 km",
  km50: "~50 km",
};

const EXPLANATION: Record<Granularity, string> = {
  exact:
    "Photos appear at the coordinates recorded by your camera. Anyone can see the precise spot each was taken.",
  km1: "Photos appear somewhere within about a kilometer — enough to show the neighborhood, not the address.",
  km10: "Photos appear within about ten kilometers — roughly the town or district, not the street.",
  km50: "Photos appear within about fifty kilometers — the region only.",
};

/** Preview circle diameters, in the design's preview scale. `exact` draws a pin
 *  rather than an area, which is the point of the control. */
const PREVIEW_SIZE: Record<Exclude<Granularity, "exact">, number> = {
  km1: 62,
  km10: 132,
  km50: 230,
};

export interface GranularitySelectorProps {
  value: Granularity;
  onChange: (value: Granularity) => void;
  /** True when the voyage has no located photos — there is nothing to coarsen,
   *  so the steps are inert and `disabledReason` says why. */
  disabled?: boolean;
  disabledReason?: string;
  /** Server-supplied labels, keyed by granularity. Falls back to
   *  `GRANULARITY_LABEL`. */
  labels?: Partial<Record<Granularity, string>>;
  /** Names the group of steps for assistive technology. */
  label?: string;
}

export function GranularitySelector({
  value,
  onChange,
  disabled = false,
  disabledReason,
  labels,
  label = "Location precision",
}: GranularitySelectorProps) {
  return (
    <div className="pv-gran">
      <div
        className="pv-gran-steps"
        role="group"
        aria-label={label}
        // Mirrors the design file, which dims the disabled set inline;
        // `design/components.css` has no disabled treatment for
        // `.pv-gran-step` or `.pv-gran-steps` yet. The `disabled` attribute
        // below is what actually makes them inert.
        style={disabled ? { opacity: 0.5 } : undefined}
      >
        {GRANULARITIES.map((step) => (
          <button
            key={step}
            type="button"
            className="pv-gran-step"
            aria-pressed={value === step}
            disabled={disabled}
            onClick={() => onChange(step)}
          >
            {labels?.[step] ?? GRANULARITY_LABEL[step]}
          </button>
        ))}
      </div>

      <div className="pv-gran-preview pv-map">
        {value === "exact" ? (
          <Pin label="A photograph at its exact recorded position" left="50%" top="52%" />
        ) : (
          <MapArea
            left="50%"
            top="52%"
            size={PREVIEW_SIZE[value]}
            label={`The area a viewer could infer at ${labels?.[value] ?? GRANULARITY_LABEL[value]}`}
          />
        )}
        <Attribution />
      </div>

      <p className="pv-gran-explain">{EXPLANATION[value]}</p>

      {disabled && disabledReason != null && (
        <p className="pv-xs pv-muted">{disabledReason}</p>
      )}

      <div className="pv-gran-guarantee">
        <ShieldCheckIcon size={16} />
        <div>
          <strong>Location data is removed from the files themselves.</strong> Images
          served by PinVoyager are re-encoded with all EXIF metadata stripped —
          including GPS, timestamp, and camera model. Your original upload is never
          served.
        </div>
      </div>
    </div>
  );
}
