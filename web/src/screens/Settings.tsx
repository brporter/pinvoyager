/** Current profile and provider-linking controls. */
import { Link } from "react-router";
import { useMutation } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { isApiError, NetworkError, updateDisplayName } from "../api/client";
import type { LinkedProvider } from "../api/types";
import { useAuth } from "../auth/useAuth";
import { Avatar, Button, EmptyState, PROVIDERS, Skeleton, TextField, Toast } from "../components";

function messageOf(error: unknown): string {
  if (error instanceof NetworkError) {
    return "The request could not be sent. Check your connection and try again.";
  }
  if (!isApiError(error)) return "Something went wrong.";
  if (error.status >= 500) {
    return error.requestId === null
      ? "The server failed. Please try again."
      : `The server failed. Please try again, quoting reference ${error.requestId}.`;
  }
  return error.message;
}

export function Settings() {
  const { status, user, identities, error, refresh } = useAuth();

  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-6)" }}>
      <div className="pv-stack-6" style={{ maxWidth: 640, margin: "0 auto" }}>
        <h1 className="pv-h2">Settings</h1>

        {status === "loading" && <LoadingSettings />}

        {status === "failed" && (
          <EmptyState
            tone="danger"
            title="Your settings could not be loaded"
            action={
              <Button variant="secondary" onClick={() => void refresh()}>
                Try again
              </Button>
            }
          >
            {messageOf(error)}
          </EmptyState>
        )}

        {status !== "loading" && status !== "failed" && user === null && (
          <EmptyState title="You are signed out">Sign in again to reach your settings.</EmptyState>
        )}

        {user !== null && (
          <>
            <ProfileSection
              displayName={user.display_name}
              avatarUrl={user.avatar_url}
              onSaved={() => void refresh()}
            />
            <ProvidersSection identities={identities} />
          </>
        )}
      </div>
    </div>
  );
}

/** Shaped like what is coming, so the page does not jump when it arrives. */
function LoadingSettings() {
  return (
    <div className="pv-stack-4" aria-busy="true">
      <span className="pv-sr" role="status">
        Loading your settings…
      </span>
      <Skeleton height={180} />
      <Skeleton height={120} />
      <Skeleton height={260} />
    </div>
  );
}

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section
      className="pv-card"
      style={{
        padding: "var(--pv-space-5)",
      }}
    >
      <h2
        className="pv-h3"
        style={{
          fontSize: "var(--pv-text-base)",
          marginBottom: description === undefined ? "var(--pv-space-4)" : "var(--pv-space-2)",
        }}
      >
        {title}
      </h2>
      {description !== undefined && (
        <p className="pv-xs pv-muted" style={{ margin: "0 0 var(--pv-space-4)" }}>
          {description}
        </p>
      )}
      {children}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

function ProfileSection({
  displayName,
  avatarUrl,
  onSaved,
}: {
  displayName: string | null;
  avatarUrl: string | null;
  onSaved: () => void;
}) {
  const [value, setValue] = useState(displayName ?? "");

  // A refresh elsewhere can change the stored name under this field. Adopting
  // it is right only while the field is untouched, which is what comparing
  // against the last-seen server value gives.
  const [lastSeen, setLastSeen] = useState(displayName);
  if (lastSeen !== displayName) {
    setLastSeen(displayName);
    setValue(displayName ?? "");
  }

  const save = useMutation({
    mutationFn: updateDisplayName,
    retry: false,
    onSuccess: onSaved,
  });

  const trimmed = value.trim();
  const dirty = trimmed !== (displayName ?? "");

  return (
    <Section title="Profile">
      <div className="pv-inline-3" style={{ marginBottom: "var(--pv-space-4)" }}>
        <Avatar name={displayName} src={avatarUrl} size="lg" />
        <p className="pv-xs pv-subtle" style={{ margin: 0, minWidth: 0 }}>
          {/* The design draws a "Change picture" button. There is nowhere to
              put a picture — PinVoyager stores none and the avatar comes from
              whichever provider supplied one — so the explanation is shown
              without a control that could not work. */}
          Your picture comes from your sign-in provider. Initials are used when a provider does not
          supply one.
        </p>
      </div>

      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (dirty) save.mutate(trimmed);
        }}
      >
        <TextField
          label="Display name"
          value={value}
          onChange={(event) => setValue(event.target.value)}
          help="Unique public URL: /u/your-name. Use 3–30 letters, digits, or hyphens; start with a letter and end with a letter or digit."
          error={save.isError ? messageOf(save.error) : undefined}
        />

        {displayName !== null && (
          <p className="pv-xs">
            <Link to={`/u/${encodeURIComponent(displayName)}`}>View your profile</Link>
          </p>
        )}

        <div style={{ marginTop: "var(--pv-space-4)" }}>
          <Button type="submit" variant="secondary" disabled={!dirty} loading={save.isPending}>
            Save name
          </Button>
          {save.isSuccess && !dirty && (
            <span
              className="pv-xs pv-muted"
              role="status"
              style={{ marginLeft: "var(--pv-space-3)" }}
            >
              Saved.
            </span>
          )}
        </div>
      </form>
    </Section>
  );
}

function ProvidersSection({ identities }: { identities: readonly LinkedProvider[] }) {
  const linked = new Map(identities.map((identity) => [identity.provider, identity]));
  const names = { google: "Google", microsoft: "Microsoft", apple: "Apple", facebook: "Facebook" };
  return (
    <Section
      title="Sign-in methods"
      description="Any linked provider signs you in to this same account. We store no passwords."
    >
      {linked.size === 0 && (
        <Toast tone="warning" title="No sign-in methods are listed">
          Reload the page before changing anything here.
        </Toast>
      )}
      <ul style={{ listStyle: "none", padding: 0 }}>
        {PROVIDERS.map((provider) => (
          <li key={provider} className="pv-inline-3" style={{ paddingBlock: "var(--pv-space-3)" }}>
            <span>{names[provider]}</span>
            {linked.has(provider) ? (
              <span className="pv-xs pv-subtle">Linked</span>
            ) : (
              // Linking redirects to the provider, so it uses a full-page form POST.
              <form method="post" action={`/auth/link/${provider}?redirect_to=%2Fsettings`}>
                <Button type="submit" variant="secondary">
                  Link
                </Button>
              </form>
            )}
          </li>
        ))}
      </ul>
    </Section>
  );
}
