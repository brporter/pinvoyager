/**
 * The chrome for the sign-in and callback screens.
 *
 * Deliberately not `AppShell`. `design/screens/sign-in.html` draws a desktop
 * bar carrying the brand and nothing else, and a mobile frame with no bar at
 * all — no navigation links, no tab bar, no "Sign in" button. A screen whose
 * whole purpose is one decision should not also offer four ways to leave it,
 * and the tab bar in particular would offer tabs that route back here.
 *
 * The centring, padding and sunken background are `.si-shell` from that file.
 * They are inline rather than a class because they belong to these two screens
 * rather than to the design system, which is exactly how the design file itself
 * treats them.
 */

import { Link, Outlet } from "react-router";

import { PinIcon } from "../components/icons";
import { useIsDesktop } from "./useIsDesktop";

export function AuthShell() {
  const desktop = useIsDesktop();

  return (
    <div style={{ minHeight: "100%", display: "flex", flexDirection: "column" }}>
      {desktop ? (
        <nav className="pv-topnav" aria-label="Main">
          <Link className="pv-brand" to="/">
            <PinIcon className="pv-brand-mark" size={22} />
            PinVoyager
          </Link>
        </nav>
      ) : null}

      <main
        style={{
          flex: 1,
          display: "grid",
          placeItems: "center",
          padding: "var(--pv-space-6)",
          background: "var(--pv-bg-subtle)",
        }}
      >
        <Outlet />
      </main>
    </div>
  );
}
