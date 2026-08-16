/**
 * Where the four navigation destinations point, signed in and signed out.
 *
 * ## The one genuinely ambiguous mapping
 *
 * `design/components/navigation.html` names four tabs — Discover, Voyages,
 * Upload, Profile — and the fixed route table in `App.tsx` has no list route to
 * hang "Voyages" on: a user's voyages are listed on their profile,
 * `/u/{userId}`, which is also the obvious target for "Profile". One of the two
 * has to go somewhere else.
 *
 * "Voyages" takes `/u/{me.id}`, matching the desktop bar's second link, which
 * the design labels "My voyages" and points at the same place. "Profile" takes
 * `/settings`, because the account screen is otherwise unreachable from the
 * mobile shell — the desktop equivalent is the avatar menu, and a phone has no
 * avatar menu to put it behind.
 *
 * The cost is visible and worth naming: `design/screens/profile.html` marks the
 * Profile tab as current on the owner's own profile, whereas here the Voyages
 * tab lights up there instead. Resolving it properly means a design decision
 * about what "Voyages" is, which is not this agent's to make.
 *
 * ## Signed out
 *
 * Three of the four become sign-in links carrying the destination, rather than
 * disappearing. Every published voyage is world-readable, so a signed-out
 * visitor is an ordinary user of this site; hiding three quarters of the
 * navigation from them makes the product look smaller than it is and hides
 * where signing in would actually get them.
 */

import type { CurrentUser } from "../api/types";
import { signInPath } from "../auth/redirect";

export interface NavDestination {
  key: "discover" | "voyages" | "upload" | "profile";
  label: string;
  to: string;
  /** `NavLink` matches by prefix unless told otherwise; only `/` needs it. */
  end: boolean;
}

/**
 * The tab bar's four destinations, in drawn order.
 *
 * `user` is null for a signed-out visitor and while the session is still being
 * read — the loading case lands on the sign-in variants, which is harmless
 * because the answer arrives in a single round trip and nothing has been
 * navigated to in the meantime.
 */
export function tabDestinations(user: CurrentUser | null): NavDestination[] {
  return [
    { key: "discover", label: "Discover", to: "/", end: true },
    {
      key: "voyages",
      label: "Voyages",
      // No `redirect_to`: the destination depends on the account id, which is
      // exactly what is not known yet. Sending `/u/…` for a user we cannot name
      // would be a guess, and the server's allowlist refuses `/u` anyway.
      to: user === null ? signInPath(null) : `/u/${user.id}`,
      end: false,
    },
    {
      key: "upload",
      label: "Upload",
      to: user === null ? signInPath("/upload") : "/upload",
      end: false,
    },
    {
      key: "profile",
      label: "Profile",
      to: user === null ? signInPath("/settings") : "/settings",
      end: false,
    },
  ];
}

/**
 * The desktop bar's links. Two when signed in, one when not — the design drops
 * "My voyages" from the signed-out bar because there is nothing behind it.
 */
export function topNavLinks(user: CurrentUser | null): NavDestination[] {
  const discover: NavDestination = { key: "discover", label: "Discover", to: "/", end: true };

  if (user === null) {
    return [discover];
  }

  return [
    discover,
    { key: "voyages", label: "My voyages", to: `/u/${user.id}`, end: false },
  ];
}
