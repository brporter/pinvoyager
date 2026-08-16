/**
 * Route table and application shell.
 *
 * CONTRACT — the route paths below are fixed; the screens and the shell are
 * owned by the routing agent and the screens agent.
 *
 * Every screen in `design/screens/` has a route here. The design is the
 * specification: if a screen exists there, it has a route, and if a route
 * exists here, there is a design for it.
 *
 * | path                  | design file                       | auth           |
 * |-----------------------|-----------------------------------|----------------|
 * | `/`                   | screens/discover.html             | public         |
 * | `/voyages/:id`        | screens/voyage-detail.html        | public if published |
 * | `/voyages/:id/edit`   | screens/voyage-edit.html          | owner          |
 * | `/upload`             | screens/upload-flow.html          | required       |
 * | `/u/:userId`          | screens/profile.html              | public         |
 * | `/settings`           | screens/settings.html             | required       |
 * | `/signin`             | screens/sign-in.html              | public         |
 * | `/auth/callback`      | screens/auth-states.html          | public         |
 * | anything else         | screens/not-found.html            | public         |
 *
 * Note what "public" means here: most of this application is readable without
 * an account, because every published voyage is world-readable. A signed-out
 * visitor is an ordinary user of this site, not someone being held at a gate,
 * and the navigation reflects that.
 *
 * ## Structure
 *
 * The paths above are unchanged; what surrounds them is three layout routes.
 *
 *  * `AppShell` — the top bar, and the tab bar below the `md` breakpoint. Every
 *    ordinary route, including the 404.
 *  * `RequireAuth` — the three routes that need a session. It redirects rather
 *    than refusing, carrying a validated `redirect_to`, and it is a routing
 *    convenience rather than access control: the server owns that, and
 *    `/voyages/:id/edit` is owner-only in a way the client cannot check.
 *  * `AuthShell` — sign-in and the callback, which the design draws with the
 *    brand alone and no navigation. A screen that exists to make one decision
 *    should not offer four ways to leave it, and the tab bar in particular
 *    would offer tabs that route straight back here.
 */

import { Route, Routes } from "react-router";

import { AuthCallback } from "./auth/AuthCallback";
import { AuthProvider } from "./auth/AuthProvider";
import { RequireAuth } from "./auth/RequireAuth";
import { SignIn } from "./auth/SignIn";
import { AppShell } from "./shell/AppShell";
import { AuthShell } from "./shell/AuthShell";
import { Discover } from "./screens/Discover";
import { NotFound } from "./screens/NotFound";
import { Profile } from "./screens/Profile";
import { Settings } from "./screens/Settings";
import { Upload } from "./screens/Upload";
import { VoyageDetail } from "./screens/VoyageDetail";
import { VoyageEdit } from "./screens/VoyageEdit";

export function App() {
  return (
    // Outside the router's own state so one read of `/api/me` serves the whole
    // session rather than one per route, and so navigating does not re-ask.
    <AuthProvider>
      <div className="pv-app" style={{ minHeight: "100%" }}>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/" element={<Discover />} />
            <Route path="/voyages/:id" element={<VoyageDetail />} />
            <Route path="/u/:userId" element={<Profile />} />

            <Route element={<RequireAuth />}>
              <Route path="/voyages/:id/edit" element={<VoyageEdit />} />
              <Route path="/upload" element={<Upload />} />
              <Route path="/settings" element={<Settings />} />
            </Route>

            <Route path="*" element={<NotFound />} />
          </Route>

          <Route element={<AuthShell />}>
            <Route path="/signin" element={<SignIn />} />
            <Route path="/auth/callback" element={<AuthCallback />} />
          </Route>
        </Routes>
      </div>
    </AuthProvider>
  );
}
