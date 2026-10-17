/**
 * Missing-route, unavailable-voyage, and server-error states.
 * Missing and inaccessible voyages use the same message to preserve draft
 * privacy. Server failures show a reference ID and offer retry when supplied.
 */

import type { ReactNode } from "react";
import { Link } from "react-router";

import { Button, EmptyState, ErrorCircleIcon } from "../components";

/** Centers an empty state below the shell with a minimum height of 60vh. */
function ErrorPage({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        minHeight: "60vh",
        display: "grid",
        placeItems: "center",
        padding: "var(--pv-space-6)",
      }}
    >
      <div style={{ width: "100%" }}>{children}</div>
    </div>
  );
}

/** Missing voyage icon. */
function BrokenPinIcon({ size = 44 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M12 21s-7-6.2-7-11a7 7 0 0 1 11.6-5.3" />
      <path d="M19 10c0 1.2-.4 2.4-1 3.6" />
      <path d="m3 3 18 18" />
    </svg>
  );
}

/** Missing photograph icon. */
function BrokenImageIcon({ size = 44 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="m3 16 5-4 4 3 3-2 6 5" />
      <path d="m3 3 18 18" />
    </svg>
  );
}

/** Fallback for an unmatched application route. */
export function NotFound() {
  return (
    <ErrorPage>
      <EmptyState
        icon={<BrokenPinIcon />}
        title="Nothing here"
        action={
          <Link className="pv-btn pv-btn--primary" to="/">
            Go to Discover
          </Link>
        }
      >
        This page does not exist. It may have been a mistyped address, or a link that has since
        moved.
      </EmptyState>
    </ErrorPage>
  );
}

/** Shared unavailable state for missing voyages and foreign drafts. */
export function VoyageUnavailable() {
  return (
    <ErrorPage>
      <EmptyState
        icon={<BrokenImageIcon />}
        title="This voyage is no longer available"
        action={
          <Link className="pv-btn pv-btn--secondary" to="/">
            Browse other voyages
          </Link>
        }
      >
        It may have been deleted or made private by the person who created it.
      </EmptyState>
    </ErrorPage>
  );
}

export interface ErrorScreenProps {
  /** Overrides "Something went wrong" where a screen can say what failed —
   *  "Voyages could not be loaded" reads better inside a page that otherwise
   *  rendered. */
  title?: ReactNode;
  /** The `x-request-id` of the failed request. Rendered verbatim and nothing
   *  else is: it is the only thread from what the user saw to what the server
   *  logged, and it is safe to show precisely because it explains nothing. */
  reference?: string | null;
  /** Omitted when there is nothing to retry, in which case the way onward is
   *  Discover instead. */
  onRetry?: () => void;
  /** Set when this is standing in for the whole screen rather than for one
   *  region of it; adds the vertical centring. */
  standalone?: boolean;
}

/** Displays a failure with optional reference ID and retry action. */
export function ErrorScreen({
  title = "Something went wrong",
  reference,
  onRetry,
  standalone = false,
}: ErrorScreenProps) {
  const body = (
    <EmptyState
      tone="danger"
      icon={<ErrorCircleIcon size={44} strokeWidth={1.4} />}
      title={title}
      action={
        onRetry === undefined ? (
          <Link className="pv-btn pv-btn--primary" to="/">
            Go to Discover
          </Link>
        ) : (
          <Button variant="primary" onClick={onRetry}>
            Try again
          </Button>
        )
      }
    >
      This is our fault, not yours. Trying again often works.
      {reference != null && reference !== "" && (
        <>
          <br />
          <br />
          <span className="pv-xs pv-subtle">Reference {reference}</span>
        </>
      )}
    </EmptyState>
  );

  return standalone ? <ErrorPage>{body}</ErrorPage> : body;
}
