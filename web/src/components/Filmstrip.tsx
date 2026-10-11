/** Filmstrip uses the shared runtime styles and accessible native controls. */

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
