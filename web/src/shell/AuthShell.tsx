/** Centered authentication layout with shared site identity. */

import { Outlet } from "react-router";

import { Brand } from "../components/Brand";
import { SiteFooter } from "./SiteFooter";

export function AuthShell() {
  return (
    <div style={{ minHeight: "100%", display: "flex", flexDirection: "column" }}>
      <nav className="pv-topnav" aria-label="Main">
        <Brand />
      </nav>

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

      <SiteFooter />
    </div>
  );
}
