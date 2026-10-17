/** Displays an icon, heading, explanation, and optional action for an empty region. */

import type { ReactNode } from "react";

import { cx } from "./cx";

export interface EmptyStateProps {
  /** An icon from `./icons`, sized 40 as the design draws it. */
  icon?: ReactNode;
  title: ReactNode;
  children?: ReactNode;
  /** Usually a single `Button`. */
  action?: ReactNode;
  /** Tints the icon for an error state. */
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
