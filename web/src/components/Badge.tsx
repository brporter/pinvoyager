/** Renders a text badge with the selected tone and shared styles. */

import type { ReactNode } from "react";

import { cx } from "./cx";

export type BadgeTone = "neutral" | "draft" | "public" | "unplaced" | "accent";

export interface BadgeProps {
  tone?: BadgeTone;
  children: ReactNode;
  className?: string;
}

export function Badge({ tone = "neutral", children, className }: BadgeProps) {
  return (
    <span className={cx("pv-badge", tone !== "neutral" && `pv-badge--${tone}`, className)}>
      {children}
    </span>
  );
}
