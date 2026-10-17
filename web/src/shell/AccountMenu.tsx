/** Accessible account menu for profile navigation, settings, and sign-out. */

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
    // Return to Discover after the sign-out action resolves, including when
    // AuthProvider suppresses a failed logout request.
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
