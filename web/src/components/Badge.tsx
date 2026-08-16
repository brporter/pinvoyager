/**
 * Badge — from `design/components/feedback.html`.
 *
 * The draft badge matters more than it looks. A voyage is world-readable the
 * moment it is published, and this is the persistent reminder of which side of
 * that line the current voyage sits on.
 */

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
