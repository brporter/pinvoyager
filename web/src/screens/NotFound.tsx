/**
 * 404 and error states — implements `design/screens/not-found.html`.
 *
 * The design draws three situations that look similar and are not:
 *
 *  * **An address that never existed.** {@link NotFound} — the `*` route.
 *  * **A voyage that is gone.** {@link VoyageUnavailable} — and the copy is
 *    deliberately non-committal. "It may have been deleted or made private by
 *    the person who created it" covers a genuinely removed voyage *and* a draft
 *    the visitor must not learn about, using one message for both so the two
 *    stay indistinguishable. The server makes the same choice by answering 404
 *    rather than 403: a "forbidden" would confirm that a voyage exists at that
 *    address and let anyone enumerate unpublished work by probing ids. It
 *    follows that this screen cannot offer a "request access" affordance
 *    without undoing the whole arrangement, and it does not.
 *  * **A server error.** {@link ErrorScreen} — a reference id and nothing else.
 *    The user gets something concrete to quote; the stack trace and the real
 *    cause stay in the server log, where the service's error type puts them
 *    against the same `x-request-id`.
 *
 * All three say what happened, and all three offer a way onward — Discover,
 * browse, or retry. A dead end with no next action is where sessions end.
 *
 * ## Two notes on the components used
 *
 * `EmptyState` from the design system renders its title as an `h3`, while the
 * design file draws these three as `<h1 class="pv-empty-title">`. The class and
 * therefore the appearance are identical; the heading level is not, so these
 * pages have no `h1`. Changing that means giving `EmptyState` a heading-level
 * prop, which belongs to the component library rather than to a screen.
 *
 * The two crossed-through icons the design draws here — a struck-out pin and a
 * struck-out photograph — are not in `src/components/icons.tsx`. They are
 * inlined below, copied path-for-path from the design file rather than
 * redrawn, and they belong in that module the moment it is being edited for
 * another reason.
 */

import type { ReactNode } from "react";
import { Link } from "react-router";

import { Button, EmptyState, ErrorCircleIcon } from "../components";

/**
 * Centers one empty state in the space the shell gives a screen.
 *
 * `min-height: 60vh` rather than the full viewport: the top bar and, on a
 * phone, the tab bar are already using some of it, and a `100vh` block inside
 * them is how a page acquires a scrollbar that scrolls nothing.
 */
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

/** The struck-out pin from `design/screens/not-found.html`. */
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

/** The struck-out photograph from `design/screens/not-found.html`. */
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

/**
 * An address that never existed — the `*` route.
 *
 * Note what it does not claim. "It may have been a mistyped address, or a link
 * that has since moved" is true of a typo and of a route that was renamed, and
 * neither reading tells anyone anything about what does exist.
 */
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
        This page does not exist. It may have been a mistyped address, or a link that has
        since moved.
      </EmptyState>
    </ErrorPage>
  );
}

/**
 * A voyage that is gone, or a draft belonging to somebody else.
 *
 * One screen for both, because the server returns one status for both. Do not
 * make this message more specific: the moment it distinguishes "deleted" from
 * "not yours to see", the 404-instead-of-403 rule that keeps unpublished work
 * unenumerable has been undone in the interface instead of the API.
 */
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

/**
 * A 5xx, or any failure a retry might fix.
 *
 * "This is our fault, not yours" costs a line and buys a retry. A 500 following
 * an upload otherwise leaves people assuming they did something wrong and
 * hesitant to try again.
 */
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
