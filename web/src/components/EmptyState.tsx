/**
 * EmptyState — from `design/components/feedback.html`.
 *
 * Empty and error states are the components most often left until last and most
 * often improvised into a spinner. The design specifies four of them — no
 * voyages, no located photos, nothing to discover, and a load failure — and
 * they all share this shape: icon, title, a sentence explaining what happened,
 * and one action.
 *
 * On the error variant, note what the design's body text does and does not say.
 * It quotes a request reference and nothing else. That pairing is deliberate:
 * it gives the user something concrete to report while keeping stack traces and
 * internal messages on the server, where the service's error type logs the real
 * cause against the same `x-request-id`.
 */

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
   *  file sets that colour inline; `design/components.css` has no
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
