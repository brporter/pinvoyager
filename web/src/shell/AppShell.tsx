/**
 * The application chrome every ordinary route renders inside.
 *
 * Top bar above, content between, tab bar below on mobile. The tab bar is
 * omitted rather than hidden at desktop widths, and the desktop navigation
 * links are omitted rather than hidden on mobile, so there is exactly one set
 * of navigation controls in the DOM at any width.
 */

import { Outlet } from "react-router";

import { SkipLink } from "./SkipLink";
import { TabBar } from "./TabBar";
import { TopNav } from "./TopNav";
import { SiteFooter } from "./SiteFooter";
import { useIsDesktop } from "./useIsDesktop";

const MAIN_ID = "pv-main";

export function AppShell() {
  const desktop = useIsDesktop();

  return (
    <div style={{ minHeight: "100%", display: "flex", flexDirection: "column" }}>
      <SkipLink targetId={MAIN_ID} />
      <TopNav desktop={desktop} />

      {/* `tabIndex={-1}` so the skip link can actually move focus here; a
          fragment jump to a non-focusable element moves the scroll position
          and leaves the tab order where it was. */}
      <main id={MAIN_ID} tabIndex={-1} style={{ flex: 1, minHeight: 0, outline: "none" }}>
        <Outlet />
      </main>

      <SiteFooter />

      {desktop ? null : <TabBar />}
    </div>
  );
}
