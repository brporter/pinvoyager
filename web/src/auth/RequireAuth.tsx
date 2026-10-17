/**
 * Protects routes that require a session. Shows a waiting state during
 * session loading and redirects anonymous or failed states to sign-in with a
 * validated return path. The API separately enforces access and ownership.
 */

import { Navigate, Outlet, useLocation } from "react-router";

import { signInPath } from "./redirect";
import { useAuth } from "./useAuth";

export function RequireAuth() {
  const { status } = useAuth();
  const location = useLocation();

  if (status === "loading") {
    // Wait for the session read before deciding whether to redirect.
    return <VerifyingSession />;
  }

  if (status === "authenticated") {
    return <Outlet />;
  }

  // Redirect anonymous and failed session reads to sign-in.
  // Validate the current location before including it as a sign-in return path.
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
