/**
 * PhotoGrid — from `design/components/photo-primitives.html`.
 *
 * `auto-fill` with a 150px minimum, so the column count follows the container
 * rather than a breakpoint. Fixed aspect ratios on the children keep the grid
 * from going ragged and stop the page reflowing as images load.
 */

import type { ReactNode } from "react";

import { cx } from "./cx";

export interface PhotoGridProps {
  children: ReactNode;
  /** Names the group for a screen reader, e.g. "Photos in this voyage". */
  label?: string;
  className?: string;
}

export function PhotoGrid({ children, label, className }: PhotoGridProps) {
  return (
    <div
      className={cx("pv-photo-grid", className)}
      role={label != null ? "group" : undefined}
      aria-label={label}
    >
      {children}
    </div>
  );
}
