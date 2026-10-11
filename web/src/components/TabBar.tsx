/** TabBar uses the shared runtime styles and accessible native controls. */

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
const DEFAULT_TABS: TabItem[] = [
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
