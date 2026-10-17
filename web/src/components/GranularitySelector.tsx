import { GRANULARITIES, GRANULARITY_LABEL } from "./granularity";
/** Renders precision choices and an optional preview, including an unselected state. */

import type { Granularity } from "../api/types";

import { LazyPrecisionPreview } from "../map/LazyPrecisionPreview";
import { ShieldCheckIcon } from "./icons";

const EXPLANATION: Record<Granularity, string> = {
  exact:
    "Photos appear at the coordinates recorded by your camera. Anyone can see the precise spot each was taken.",
  km1: "Photos appear somewhere within about a kilometer — enough to show the neighborhood, not the address.",
  km10: "Photos appear within about ten kilometers — roughly the town or district, not the street.",
  km50: "Photos appear within about fifty kilometers — the region only.",
};

export interface GranularitySelectorProps {
  /** Null leaves every step unselected until the user explicitly chooses. */
  value: Granularity | null;
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
        // Dim disabled choices; each button's disabled attribute prevents activation.
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

      <LazyPrecisionPreview precision={value} />
      {value === null && (
        <p className="pv-gran-explain">Select a precision to preview its effect.</p>
      )}

      <p className="pv-gran-explain">
        {value === null
          ? "Choose the public location precision for this voyage."
          : EXPLANATION[value]}
      </p>

      {disabled && disabledReason != null && <p className="pv-xs pv-muted">{disabledReason}</p>}

      <div className="pv-gran-guarantee">
        <ShieldCheckIcon size={16} />
        <div>
          <strong>Location data is removed from the files themselves.</strong> Images served by
          PinVoyager are re-encoded with all EXIF metadata stripped — including GPS, timestamp, and
          camera model. Your original upload is never served.
        </div>
      </div>
    </div>
  );
}
