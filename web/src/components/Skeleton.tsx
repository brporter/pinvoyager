/** Renders a decorative loading placeholder hidden from assistive technology. */

import { cx } from "./cx";

export interface SkeletonProps {
  /** Any CSS length. The design specimens use "60%" and "40%". */
  width?: string | number;
  height?: string | number;
  /** Design-system layout classes, e.g. `pv-photo pv-photo--square`. */
  className?: string;
}

export function Skeleton({ width, height, className }: SkeletonProps) {
  return (
    <div className={cx("pv-skeleton", className)} style={{ width, height }} aria-hidden="true" />
  );
}
