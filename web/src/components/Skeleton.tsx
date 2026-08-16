/**
 * Skeleton — from `design/components/feedback.html`.
 *
 * Skeletons match the shape of what is coming, so the layout does not jump when
 * content arrives. Under `prefers-reduced-motion` the shimmer stops and the
 * block goes flat — handled by the design system, not here.
 *
 * A skeleton is always `aria-hidden`. It represents the absence of content, and
 * reading out a row of empty boxes is worse than silence. The region that owns
 * the skeletons should carry `aria-busy="true"` instead, so a screen reader is
 * told the page is loading exactly once.
 */

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
