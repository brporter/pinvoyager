/** Renders a labeled grid of photos using the shared responsive layout. */

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
