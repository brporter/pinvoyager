/** EmptyState uses the shared runtime styles and accessible native controls. */

import type { ReactNode } from "react";

import { cx } from "./cx";

export interface EmptyStateProps {
  /** An icon from `./icons`, sized 40 as the design draws it. */
  icon?: ReactNode;
  title: ReactNode;
  children?: ReactNode;
  /** Usually a single `Button`. */
  action?: ReactNode;
  /** `danger` tints the icon for the "could not be loaded" variant. The design
   *  file sets that color inline; `design/components.css` has no
   *  `.pv-empty-icon--danger` modifier yet, so the inline style is mirrored
   *  here rather than invented elsewhere. */
  tone?: "default" | "danger";
  className?: string;
}

export function EmptyState({
  icon,
  title,
  children,
  action,
  tone = "default",
  className,
}: EmptyStateProps) {
  return (
    <div className={cx("pv-empty", className)}>
      {icon != null && (
        <div
          className="pv-empty-icon"
          style={tone === "danger" ? { color: "var(--pv-danger)" } : undefined}
        >
          {icon}
        </div>
      )}
      <h3 className="pv-empty-title">{title}</h3>
      {children != null && <p className="pv-empty-body">{children}</p>}
      {action}
    </div>
  );
}
