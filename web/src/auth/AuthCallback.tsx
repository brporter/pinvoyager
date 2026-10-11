/** Displays callback outcomes and resumes the validated return path. */

import { Navigate, useNavigate, useSearchParams } from "react-router";
import type { ReactNode } from "react";

import { signInUrl } from "../api/client";
import { Button } from "../components/Button";
import type { Provider } from "../components/ProviderButton";
import { AuthCard } from "./AuthCard";
import { providerNameOf, providerOf } from "./providers";
import { REDIRECT_PARAM, destinationOr, signInPath } from "./redirect";
import { useAuth } from "./useAuth";

type Outcome = "verifying" | "canceled" | "conflict" | "failed";

/**
 * The outcome named in the query string, or `null` when none was.
 *
 * `error=access_denied` is accepted alongside `status=canceled` because that
 * is the parameter OAuth providers themselves use, and a callback proxied
 * straight through would carry it.
 */
function namedOutcome(params: URLSearchParams): Outcome | null {
  const raw = (params.get("status") ?? params.get("error"))?.trim().toLowerCase();

  if (raw === undefined || raw === "") {
    return null;
  }

  // `"cancelled"` is the spelling this app emitted before, and a redirect
  // issued by an older server can still be in flight during a deploy.
  if (raw === "canceled" || raw === "cancelled" || raw === "access_denied") {
    return "canceled";
  }

  if (raw === "conflict" || raw === "already_linked") {
    return "conflict";
  }

  if (raw === "verifying" || raw === "pending") {
    return "verifying";
  }

  // Anything else the provider or the API chose to say. Not rendered — see the
  // module comment on why every failure gets one screen.
  return "failed";
}

export function AuthCallback() {
  const { status, user } = useAuth();
  const [params] = useSearchParams();
  const navigate = useNavigate();

  const rawProvider = params.get("provider");
  const provider = providerOf(rawProvider);
  const providerName = providerNameOf(rawProvider);
  const reference = params.get("ref") ?? params.get("request_id");
  const destination = destinationOr(params.get(REDIRECT_PARAM));

  const named = namedOutcome(params);

  // Only when nothing in the URL said otherwise. A conflict or a cancellation
  // can arrive in a browser that already holds a valid session — linking a
  // second provider happens while signed in — and bouncing those away would
  // discard the only explanation the user gets.
  if (named === null && status === "authenticated") {
    return <Navigate to={destination} replace />;
  }

  // With no outcome in the URL the session is the answer. `loading` is the
  // verifying state rather than a failure: the read is in flight, and calling
  // it a failure for the few hundred milliseconds it takes would flash an
  // error screen at a user who is about to be signed in.
  const outcome: Outcome = named ?? (status === "loading" ? "verifying" : "failed");

  if (outcome === "verifying") {
    return (
      <AuthCard variant="state" labelledBy="callback-heading">
        <div style={{ marginBottom: "var(--pv-space-4)" }}>
          <span
            className="pv-spinner"
            aria-hidden="true"
            style={{
              width: "28px",
              height: "28px",
              borderWidth: "3px",
              color: "var(--pv-accent)",
              display: "inline-block",
            }}
          />
        </div>
        <h1 className="pv-h3" id="callback-heading">
          Signing you in…
        </h1>
        <p className="pv-sm pv-muted" style={{ marginTop: "var(--pv-space-2)" }} role="status">
          {providerName === null
            ? "Verifying your account."
            : `Verifying your ${providerName} account.`}
        </p>
      </AuthCard>
    );
  }

  if (outcome === "canceled") {
    return (
      <StateCard
        heading="Sign-in canceled"
        // Neutral, and gray rather than red: declining at the provider is a
        // normal choice, not an error. "Nothing was created" is the
        // reassurance that matters — someone who backed out wants to know they
        // did not leave an account behind.
        icon={
          <svg
            width="34"
            height="34"
            viewBox="0 0 24 24"
            fill="none"
            stroke="var(--pv-text-subtle)"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
            focusable="false"
          >
            <circle cx="12" cy="12" r="9" />
            <path d="m15 9-6 6M9 9l6 6" />
          </svg>
        }
        body={
          <>
            You declined access at {providerName ?? "the provider"}, so nothing was created. You can
            try again or use a different provider.
          </>
        }
        actions={
          <Button
            variant="primary"
            style={FULL_WIDTH}
            onClick={() => {
              void navigate(signInPath(destination), { replace: true });
            }}
          >
            Back to sign in
          </Button>
        }
      />
    );
  }

  if (outcome === "conflict") {
    return (
      <StateCard
        heading={
          providerName === null
            ? "That account is already in use"
            : `That ${providerName} ID is already in use`
        }
        icon={
          <svg
            width="34"
            height="34"
            viewBox="0 0 24 24"
            fill="none"
            stroke="var(--pv-warning)"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
            focusable="false"
          >
            <path d="M12 3 2 20h20L12 3Z" />
            <path d="M12 10v4M12 17v.01" />
          </svg>
        }
        alert
        body={
          <>
            It is linked to a different PinVoyager account. To bring it to this account, sign in to
            the other one first and unlink it there.
            <br />
            <br />
            {/* Quoted exactly, and it is a promise the server keeps: the link
                handler returns before any write and rolls back the read-only
                transaction, so neither account's rows — not even a
                `last_seen_at` — are touched. A user whose link attempt failed
                is afraid they have half-merged something, and saying this
                plainly is cheaper than any recovery flow. */}
            Nothing has been changed on either account.
          </>
        }
        actions={
          <div className="pv-stack-2">
            {provider === null || providerName === null ? null : (
              <Button
                variant="primary"
                style={FULL_WIDTH}
                onClick={() => {
                  restartAt(provider);
                }}
              >
                Sign in with {providerName} instead
              </Button>
            )}
            <Button
              variant="ghost"
              style={FULL_WIDTH}
              onClick={() => {
                // Back to where a link attempt started. A conflict can only
                // arise from linking, which begins in Settings → Sign-in
                // methods, so that is where canceling belongs.
                void navigate(user === null ? "/" : "/settings", { replace: true });
              }}
            >
              Cancel
            </Button>
          </div>
        }
      />
    );
  }

  return (
    <StateCard
      heading="We could not complete sign-in"
      icon={
        <svg
          width="34"
          height="34"
          viewBox="0 0 24 24"
          fill="none"
          stroke="var(--pv-danger)"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
          focusable="false"
        >
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v5M12 16.5v.01" />
        </svg>
      }
      alert
      body={
        <>
          The response from {providerName ?? "the provider"} could not be verified. This usually
          means the sign-in took too long — starting again normally works.
          {reference === null ? null : (
            <>
              <br />
              <br />
              {/* The `x-request-id` the failure carried. It is the only handle
                  on what actually went wrong, because the response body
                  deliberately does not say — quoting it is what makes an
                  otherwise opaque failure diagnosable from the server log. */}
              <span className="pv-xs pv-subtle">Reference {reference}</span>
            </>
          )}
        </>
      }
      actions={
        <Button
          variant="primary"
          style={FULL_WIDTH}
          onClick={() => {
            void navigate(signInPath(destination), { replace: true });
          }}
        >
          Try again
        </Button>
      }
    />
  );
}

const FULL_WIDTH = { width: "100%" } as const;

/** Restarts the whole flow at the provider that conflicted. A full page
 *  navigation for the same reason the sign-in buttons are one: the endpoint
 *  redirects to another origin, and a fetch would be refused there. */
function restartAt(provider: Provider): void {
  window.location.assign(signInUrl(provider));
}

/** The shared shape of the three settled states: icon, heading, prose,
 *  actions. `.as-card` centers all of it. */
function StateCard({
  heading,
  icon,
  body,
  actions,
  alert = false,
}: {
  heading: string;
  icon: ReactNode;
  body: ReactNode;
  actions: ReactNode;
  /** Announced immediately. True for the two failures, false for cancellation
   *  — the user chose that one and does not need it read out as a problem. */
  alert?: boolean;
}) {
  return (
    <AuthCard variant="state" labelledBy="callback-heading">
      <div style={{ marginBottom: "var(--pv-space-4)" }}>{icon}</div>
      <h1 className="pv-h3" id="callback-heading">
        {heading}
      </h1>
      <p
        className="pv-sm pv-muted"
        style={{ margin: "var(--pv-space-2) 0 var(--pv-space-5)" }}
        role={alert ? "alert" : undefined}
      >
        {body}
      </p>
      {actions}
    </AuthCard>
  );
}
