/** Connects session and route state to the navigation components. */

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
      user={user === null ? null : { displayName: user.display_name, avatarUrl: user.avatar_url }}
      actions={
        <>
          {desktop && user !== null ? (
            <Link className="pv-btn pv-btn--secondary pv-btn--sm" style={TOUCH_MIN} to="/upload">
              <PlusIcon size={14} />
              <span className="pv-btn-label">New voyage</span>
            </Link>
          ) : null}

          {user === null ? null : <AccountMenu user={user} />}

          {/* Hide sign-in while loading the session. Failed reads offer sign-in again. */}
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
