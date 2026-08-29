/**
 * The avatar in the top bar, and what it opens.
 *
 * DESIGN DEVIATION, deliberate and narrow. `design/components/navigation.html`
 * draws the signed-in bar ending in a static avatar with a `title`, and no
 * screen in `design/screens/` draws a sign-out control anywhere. Phase 6's T5
 * nonetheless requires that logout returns to the signed-out state, so it needs
 * a home. This is the smallest one: the avatar becomes a disclosure with two
 * destinations behind it, and the avatar itself still looks exactly as drawn.
 *
 * It is a disclosure, not a `role="menu"`. A real menu owes the user arrow-key
 * navigation, type-ahead, and focus that never lands on an item via Tab — a
 * contract that is easy to half-implement and worse half-implemented than not
 * attempted. Two links behind a button that reports `aria-expanded` is fully
 * keyboard-operable with no bespoke key handling at all.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";

import type { CurrentUser } from "../api/types";
import { Avatar } from "../components/Avatar";
import { useAuth } from "../auth/useAuth";

/** The 32px avatar keeps its drawn size; the control around it is 44px so the
 *  hit area meets the minimum on a touch screen (T7). */
const TRIGGER_STYLE = {
  minWidth: "var(--pv-touch-min)",
  minHeight: "var(--pv-touch-min)",
  padding: 0,
} as const;

const ITEM_STYLE = {
  width: "100%",
  justifyContent: "flex-start",
  minHeight: "var(--pv-touch-min)",
} as const;

export function AccountMenu({ user }: { user: CurrentUser }) {
  const { signOut } = useAuth();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);

  const panelId = useId();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);

  // A pointer press anywhere else dismisses it. `pointerdown` rather than
  // `click`, so the panel is gone before the press lands on whatever is
  // beneath it — which is what makes dismiss-and-activate in one tap behave.
  useEffect(() => {
    if (!open) {
      return;
    }

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && containerRef.current?.contains(target) === true) {
        return;
      }
      setOpen(false);
    };

    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [open]);

  const onSignOut = useCallback(async () => {
    setOpen(false);
    await signOut();
    // Back to Discover. Staying put would leave a signed-out visitor on a route
    // the guard is about to bounce them off anyway, and that redirect would
    // hand them a sign-in screen they did not ask for.
    void navigate("/", { replace: true });
  }, [navigate, signOut]);

  const name = user.display_name;

  return (
    <div
      ref={containerRef}
      style={{ position: "relative", display: "flex" }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.stopPropagation();
          setOpen(false);
          buttonRef.current?.focus();
        }
      }}
      // Tabbing out of the last item closes it. `focusout` bubbles where `blur`
      // does not, and `relatedTarget` says where focus went.
      onBlur={(event) => {
        const next = event.relatedTarget;
        if (next instanceof Node && event.currentTarget.contains(next)) {
          return;
        }
        setOpen(false);
      }}
    >
      <button
        ref={buttonRef}
        type="button"
        className="pv-btn pv-btn--ghost pv-btn--icon"
        style={TRIGGER_STYLE}
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => {
          setOpen((wasOpen) => !wasOpen);
        }}
      >
        {/* No `label` on the Avatar: the button below already carries the
            accessible name, and a labeled image inside a labeled button is
            the same person announced twice. */}
        <Avatar name={name} src={user.avatar_url} />
        <span className="pv-sr">
          {name === null ? "Your account" : `Account — signed in as ${name}`}
        </span>
      </button>

      {open ? (
        <div
          id={panelId}
          className="pv-card pv-stack-2"
          style={{
            position: "absolute",
            top: "calc(100% + var(--pv-space-2))",
            right: 0,
            zIndex: 40,
            minWidth: "200px",
            padding: "var(--pv-space-2)",
            background: "var(--pv-bg-raised)",
            boxShadow: "var(--pv-shadow-2)",
          }}
        >
          <Link
            className="pv-btn pv-btn--ghost"
            style={ITEM_STYLE}
            to="/settings"
            onClick={() => {
              setOpen(false);
            }}
          >
            <span className="pv-btn-label">Settings</span>
          </Link>
          <button
            type="button"
            className="pv-btn pv-btn--ghost"
            style={ITEM_STYLE}
            onClick={() => {
              void onSignOut();
            }}
          >
            <span className="pv-btn-label">Sign out</span>
          </button>
        </div>
      ) : null}
    </div>
  );
}
