/**
 * The top bar, wired to the session.
 *
 * The bar itself is `components/TopNav`, which owns the markup from
 * `design/components/navigation.html`. This is the connected half: it decides
 * what the links are, and it replaces the default right-hand controls with a
 * "New voyage" action and the account disclosure, because the presentational
 * component draws a static avatar and sign-out has to live somewhere.
 *
 * The bar exists at both breakpoints — the mobile frames in `design/screens/`
 * keep a compact version of it above the content and add the tab bar below.
 * What changes is its contents: on mobile the navigation links are dropped,
 * because the tab bar underneath is already doing the navigating and repeating
 * the destinations would be two ways to reach four places on a 390px screen.
 */

import { Link, useLocation } from "react-router";

import { TopNav as TopNavBar } from "../components/TopNav";
import { PlusIcon } from "../components/icons";
import { signInPath } from "../auth/redirect";
import { useAuth } from "../auth/useAuth";
import { AccountMenu } from "./AccountMenu";
import { topNavLinks } from "./destinations";

/** Raised to the 44px minimum hit area (T7) even where the design draws the
 *  `--sm` height of 32px. The visible treatment is unchanged; only the target
 *  grows, and the bar is 56px tall so there is room for it. */
const TOUCH_MIN = { minHeight: "var(--pv-touch-min)" } as const;

export function TopNav({ desktop }: { desktop: boolean }) {
  const { status, user } = useAuth();
  const location = useLocation();

  return (
    <TopNavBar
      links={desktop ? topNavLinks(user) : []}
      user={
        user === null
          ? null
          : { displayName: user.display_name, avatarUrl: user.avatar_url }
      }
      actions={
        <>
          {desktop && user !== null ? (
            <Link className="pv-btn pv-btn--secondary pv-btn--sm" style={TOUCH_MIN} to="/upload">
              <PlusIcon size={14} />
              <span className="pv-btn-label">New voyage</span>
            </Link>
          ) : null}

          {user === null ? null : <AccountMenu user={user} />}

          {/* Suppressed only while the answer is still in flight. A "Sign in"
              button offered to someone who is already signed in — and replaced
              by their avatar a moment later — reads as the session having been
              lost, and the wait is one round trip.

              `failed` does show it. The state means "we could not find out",
              and a bar with neither an avatar nor a way in is a dead end on
              desktop: the tab bar that would otherwise rescue it is not
              rendered above the `md` breakpoint. A link is also the harmless
              direction to be wrong in — following it while signed in lands on
              a screen that immediately redirects back. */}
          {status === "anonymous" || status === "failed" ? (
            <Link
              className="pv-btn pv-btn--primary pv-btn--sm"
              style={TOUCH_MIN}
              to={signInPath(location.pathname + location.search)}
            >
              <span className="pv-btn-label">Sign in</span>
            </Link>
          ) : null}
        </>
      }
    />
  );
}
