/** Application routes, session context, and display-name requirements. */

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
            <Route path="/u/:identifier" element={<Profile />} />

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
