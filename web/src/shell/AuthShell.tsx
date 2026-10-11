/** Centered authentication layout with shared site identity. */

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
