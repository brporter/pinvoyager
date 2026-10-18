/** Provider sign-in choices and safe return navigation. */

import { useState } from "react";
import { Navigate, useSearchParams } from "react-router";

import { signInUrl } from "../api/client";
import { ProviderButton } from "../components/ProviderButton";
import { PROVIDERS } from "../components/providers";
import type { Provider } from "../components/ProviderButton";
import { PinIcon } from "../components/icons";
import { AuthCard } from "./AuthCard";
import { DevSignIn } from "./DevSignIn";
import { REDIRECT_PARAM, destinationOr } from "./redirect";
import { useAuth } from "./useAuth";

export function SignIn() {
  const { status, refresh } = useAuth();
  const [params] = useSearchParams();
  const [starting, setStarting] = useState<Provider | null>(null);

  // Validate the return destination using the same path rules as the server.
  const destination = destinationOr(params.get(REDIRECT_PARAM));

  if (status === "authenticated") {
    return <Navigate to={destination} replace />;
  }

  const start = (provider: Provider) => {
    setStarting(provider);
    // Navigate the browser so it follows the provider authorization redirects.
    window.location.assign(signInUrl(provider, destination));
  };

  return (
    <AuthCard labelledBy="signin-heading">
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: "var(--pv-space-3)",
          marginBottom: "var(--pv-space-6)",
        }}
      >
        <PinIcon size={34} strokeWidth={1.8} style={{ color: "var(--pv-accent)" }} />
        <div style={{ textAlign: "center" }}>
          <h1 className="pv-h3" id="signin-heading">
            Sign in or create an account
          </h1>
          <p className="pv-sm pv-muted" style={{ marginTop: "var(--pv-space-2)" }}>
            Choose a provider. We never see or store a password.
          </p>
        </div>
      </div>

      <div className="pv-stack-3">
        {PROVIDERS.map((provider) => (
          <ProviderButton
            key={provider}
            provider={provider}
            // Blocks a second activation while the browser is leaving. Two
            // OAuth redirects started from one page race each other, and the
            // loser's `state` is consumed by an attempt that never completes.
            loading={starting === provider}
            disabled={starting !== null && starting !== provider}
            onClick={() => {
              start(provider);
            }}
          />
        ))}
      </div>

      <p
        className="pv-subtle"
        style={{
          fontSize: "var(--pv-text-2xs)",
          textAlign: "center",
          marginTop: "var(--pv-space-5)",
          lineHeight: "var(--pv-leading-normal)",
        }}
      >
        By continuing you agree to the Terms and Privacy Policy.
        <br />
        Voyages start private. Publishing makes their photos and selected location precision public.
      </p>

      {/* Only render the development sign-in form in development builds. */}
      {import.meta.env.DEV ? (
        <DevSignIn
          onSignedIn={async () => {
            // The cookie is set; the state that reflects it is not. Re-read
            // before the redirect, or the guard on the destination route sees
            // the stale anonymous answer and bounces straight back here.
            await refresh();
          }}
        />
      ) : null}
    </AuthCard>
  );
}
