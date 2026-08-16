/**
 * Filmstrip — from `design/components/photo-primitives.html`.
 *
 * Horizontal and scroll-snapped, used inside the mobile photo sheet where
 * vertical space is scarce.
 *
 * `tabIndex={0}` is deliberate. A scroll container that only responds to a
 * pointer is unreachable by keyboard; giving it focus lets the arrow keys
 * scroll it, and the design system's focus ring makes that visible. It is
 * removed automatically when the strip is empty, because focusing an empty
 * region is a dead stop in the tab order.
 */

import type { ReactNode } from "react";
import { Children } from "react";

import { cx } from "./cx";

export interface FilmstripProps {
  children: ReactNode;
  /** Names the scroll region, e.g. "Photos". */
  label?: string;
  className?: string;
}

export function Filmstrip({ children, label, className }: FilmstripProps) {
  const empty = Children.count(children) === 0;

  return (
    <div
      className={cx("pv-filmstrip", className)}
      role={label != null ? "group" : undefined}
      aria-label={label}
      tabIndex={empty ? undefined : 0}
    >
      {children}
    </div>
  );
}
