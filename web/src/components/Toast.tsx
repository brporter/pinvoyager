/** Displays status feedback with tone, optional details, and dismissal. */

import type { ReactNode } from "react";

import { Button } from "./Button";
import { cx } from "./cx";
import {
  AlertCircleIcon,
  CheckIcon,
  CloseIcon,
  ErrorCircleIcon,
  WarningTriangleIcon,
} from "./icons";

export type ToastTone = "info" | "success" | "error" | "warning";

export interface ToastProps {
  tone?: ToastTone;
  title: ReactNode;
  /** Additional explanatory content below the title. */
  children?: ReactNode;
  onDismiss?: () => void;
  className?: string;
}

const TONE_ROLE: Record<ToastTone, "status" | "alert"> = {
  info: "status",
  success: "status",
  error: "alert",
  warning: "alert",
};

function ToneIcon({ tone }: { tone: ToastTone }) {
  switch (tone) {
    case "success":
      return <CheckIcon size={18} strokeWidth={2.5} stroke="var(--pv-success)" />;
    case "error":
      return <ErrorCircleIcon size={18} strokeWidth={2.5} stroke="var(--pv-danger)" />;
    case "warning":
      return <WarningTriangleIcon size={18} strokeWidth={2.5} stroke="var(--pv-warning)" />;
    case "info":
      return <AlertCircleIcon size={18} strokeWidth={2.5} />;
  }
}

export function Toast({ tone = "info", title, children, onDismiss, className }: ToastProps) {
  return (
    <div
      className={cx("pv-toast", tone !== "info" && `pv-toast--${tone}`, className)}
      role={TONE_ROLE[tone]}
    >
      <ToneIcon tone={tone} />
      <div>
        <div className="pv-toast-title">{title}</div>
        {children != null && <div className="pv-toast-body">{children}</div>}
      </div>
      {onDismiss != null && (
        <>
          <div className="pv-spacer" />
          <Button variant="ghost" size="sm" iconOnly aria-label="Dismiss" onClick={onDismiss}>
            <CloseIcon size={14} />
          </Button>
        </>
      )}
    </div>
  );
}
