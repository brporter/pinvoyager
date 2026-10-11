/**
 * The route guard for the three routes that need a session.
 *
 * `/upload`, `/settings` and `/voyages/:id/edit` and nothing else. Everything
 * else in this application is public, because every published voyage is
 * world-readable — a signed-out visitor is an ordinary user of this site, not
 * someone who must sign in, so wrapping routes in this by default would be
 * the wrong shape entirely.
 *
 * Note what this is and is not. It is a routing convenience: it puts a
 * signed-out user in front of the sign-in screen instead of in front of an
 * empty page that 401s. It is not access control — the server owns that, and
 * `/voyages/:id/edit` in particular is owner-only in a way this cannot check,
 * because the client does not know who owns a voyage until it has asked.
 */

import { Navigate, Outlet, useLocation } from "react-router";

import { signInPath } from "./redirect";
import { useAuth } from "./useAuth";

export function RequireAuth() {
  const { status } = useAuth();
  const location = useLocation();

  if (status === "loading") {
    // Not a redirect. Sending a user to sign-in before `/api/me` has answered
    // would bounce every signed-in user off every protected route on reload and
    // back again, which looks exactly like the session having been lost.
    return <VerifyingSession />;
  }

  if (status === "authenticated") {
    return <Outlet />;
  }

  // `failed` lands here with `anonymous`. It is the safe direction: the server
  // will refuse the request anyway, and the sign-in screen is a better place to
  // be stuck than a protected screen that cannot load its data.
  //
  // The destination is validated on the way in as well as on the way out. It is
  // built from `location`, which is whatever is in the address bar — so it is a
  // caller-supplied value like any other, and the allowlist is what stops
  // `/signin?redirect_to=https://evil.example` being a link worth sending
  // someone. `signInPath` drops anything that does not pass and the user simply
  // lands on `/` afterwards.
  const attempted = location.pathname + location.search;

  return <Navigate to={signInPath(attempted)} replace />;
}

/** The waiting state, sized so the shell does not jump when the real screen
 *  replaces it. */
function VerifyingSession() {
  return (
    <div
      style={{
        display: "grid",
        placeItems: "center",
        padding: "var(--pv-space-8)",
        minHeight: "50vh",
      }}
    >
      <p className="pv-sm pv-muted pv-inline-2" role="status">
        <span className="pv-spinner" aria-hidden="true" />
        Checking your session…
      </p>
    </div>
  );
}
