/**
 * The mobile bottom tab bar, wired to the session.
 *
 * The bar is `components/TabBar`; this supplies its four destinations, which
 * depend on who is signed in — see `destinations.ts` for the mapping and for
 * why three of the four become sign-in links rather than disappearing when
 * nobody is.
 *
 * The component's own `DEFAULT_TABS` are not used: they point at `/voyages`,
 * which is not a route in `App.tsx` and would land on the 404.
 */

import { TabBar as TabBarChrome } from "../components/TabBar";
import type { TabItem } from "../components/TabBar";
import { PersonIcon, PinIcon, SearchIcon, UploadIcon } from "../components/icons";
import { useAuth } from "../auth/useAuth";
import { tabDestinations } from "./destinations";

const ICONS = {
  discover: <SearchIcon size={22} />,
  voyages: <PinIcon size={22} />,
  upload: <UploadIcon size={22} />,
  profile: <PersonIcon size={22} />,
};

export function TabBar() {
  const { user } = useAuth();

  const items: TabItem[] = tabDestinations(user).map((tab) => ({
    label: tab.label,
    to: tab.to,
    end: tab.end,
    icon: ICONS[tab.key],
  }));

  return <TabBarChrome items={items} />;
}
