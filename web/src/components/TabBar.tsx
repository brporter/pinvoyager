/**
 * TabBar — the mobile bar from `design/components/navigation.html`.
 *
 * Four destinations, because a fifth would push the labels below a readable
 * size. Icon *and* label on every tab: an icon-only tab bar forces users to
 * learn an arbitrary pictogram vocabulary, and "Discover" versus "Voyages" is
 * not a distinction any icon pair conveys on its own.
 *
 * `.pv-tabbar` reserves `env(safe-area-inset-bottom)`. On a phone with a home
 * indicator, a bar that stops at the viewport edge puts its controls under the
 * system gesture area and taps get intercepted — invisible in a desktop
 * browser, obvious on hardware.
 *
 * Pairs with `TopNav`; see the note there about rendering only one of the two.
 */

import type { ReactNode } from "react";
import { NavLink } from "react-router";

import { PersonIcon, PinIcon, SearchIcon, UploadIcon } from "./icons";

export interface TabItem {
  label: string;
  to: string;
  icon: ReactNode;
  end?: boolean;
}

/** The four destinations the design draws, wired to the routes in `App.tsx`. */
export const DEFAULT_TABS: TabItem[] = [
  { label: "Discover", to: "/", icon: <SearchIcon size={22} />, end: true },
  { label: "Voyages", to: "/voyages", icon: <PinIcon size={22} /> },
  { label: "Upload", to: "/upload", icon: <UploadIcon size={22} /> },
  { label: "Profile", to: "/settings", icon: <PersonIcon size={22} /> },
];

export interface TabBarProps {
  items?: TabItem[];
  label?: string;
}

export function TabBar({ items = DEFAULT_TABS, label = "Main" }: TabBarProps) {
  return (
    <nav className="pv-tabbar" aria-label={label}>
      {items.map((item) => (
        <NavLink key={item.to} className="pv-tab" to={item.to} end={item.end}>
          {item.icon}
          {item.label}
        </NavLink>
      ))}
    </nav>
  );
}
