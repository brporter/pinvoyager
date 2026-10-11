/** Shared shell destinations. Signed-out actions preserve their sign-in return path. */

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

  return [discover, { key: "voyages", label: "My voyages", to: `/u/${user.id}`, end: false }];
}
