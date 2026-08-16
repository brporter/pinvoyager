/**
 * Settings — implements `design/screens/settings.html` (T9).
 *
 * Profile, default location precision, linked sign-in methods, and deletion.
 * There is no password section, no two-factor section and no email preferences,
 * because none of those exist in this architecture.
 *
 * ## The rule this screen exists to enforce
 *
 * **Unlinking the last remaining provider is refused.** Sign-in is social-only
 * and no password is stored anywhere, so a user who removes their only identity
 * has no way back into the account — and no support path we control can restore
 * it, because there is nothing to prove ownership against. The refusal is
 * therefore structural rather than a warning dialogue: with one provider linked
 * the control is `disabled`, and it is paired with a `Toast` explaining the
 * consequence and the remedy. A disabled control with no reason attached reads
 * as a bug.
 *
 * The same reasoning is why linking is *prompted* rather than merely permitted:
 * recovery is genuinely impossible here, so the interface nudges toward a
 * second provider instead of waiting for the user to work that out.
 *
 * ## Endpoints this screen needs that the service does not yet expose
 *
 * Three of the actions the design draws have no handler in `crates/api` today.
 * They are wired here to the paths the design implies, through {@link send}
 * below, so that the screen becomes functional the moment those handlers land
 * rather than having to be rewritten:
 *
 *  * `PATCH  /api/me`               — change the display name
 *  * `DELETE /auth/link/{provider}` — unlink a provider
 *  * `DELETE /api/me`               — delete the account
 *
 * Until then each of them answers 404/405 and the screen shows a real error,
 * which is the honest behaviour: the alternative is a Save button that silently
 * does nothing. `POST /auth/link/{provider}` — the one that *does* exist — is a
 * form submission rather than a fetch, because it answers with a 302 to the
 * provider's consent screen and that has to be a full-page navigation.
 *
 * The default precision control is a local preference (see
 * {@link readDefaultGranularity}). `MeResponse` carries no such field, so there
 * is nowhere on the server to put it yet.
 *
 * ## No email address appears anywhere here
 *
 * Deliberate, and a phase 3 recommendation: provider-supplied email is not
 * needed for sign-in, and storing it would create PII we would otherwise not
 * hold.
 */

import { useMutation } from "@tanstack/react-query";
import { useEffect, useId, useState } from "react";
import type { ReactNode } from "react";

import { errorFrom, isApiError, NetworkError } from "../api/client";
import type { ApiErrorBody } from "../api/client";
import type { Granularity, LinkedProvider } from "../api/types";
import { useAuth } from "../auth/useAuth";
import {
  Avatar,
  Button,
  EmptyState,
  GRANULARITIES,
  GRANULARITY_LABEL,
  PROVIDERS,
  Skeleton,
  TextField,
  Toast,
  WarningTriangleIcon,
} from "../components";
import type { Provider } from "../components";

const PROVIDER_NAME: Record<Provider, string> = {
  google: "Google",
  apple: "Apple",
  microsoft: "Microsoft",
  facebook: "Facebook",
};

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/**
 * A request for the three endpoints `src/api/client.ts` does not yet have.
 *
 * Duplicating the client's `request` is not the intent — this exists only
 * because the calls it makes have no handler yet, so adding them to the typed
 * client would be adding methods for endpoints that answer 405. Failures are
 * routed through the client's own {@link errorFrom} so they arrive as the same
 * `ApiRequestError` every other call throws, `x-request-id` included. When the
 * handlers land, these three move into the client and this function goes away.
 */
async function send(method: string, path: string, body?: unknown): Promise<void> {
  const headers = new Headers({ accept: "application/json" });
  if (body !== undefined) headers.set("content-type", "application/json");

  let response: Response;
  try {
    response = await fetch(path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      // The session is an HttpOnly cookie. Without this the request is
      // anonymous and every one of these answers 401.
      credentials: "same-origin",
    });
  } catch (cause) {
    throw new NetworkError("The request could not be sent.", cause);
  }

  if (response.ok) return;

  let parsed: ApiErrorBody = {
    code: "unknown",
    message: response.statusText || "The request failed.",
  };
  try {
    const text = await response.text();
    if (text !== "") {
      const decoded: unknown = JSON.parse(text);
      if (decoded !== null && typeof decoded === "object") {
        const candidate = decoded as Partial<ApiErrorBody>;
        parsed = {
          code: typeof candidate.code === "string" ? candidate.code : parsed.code,
          message:
            typeof candidate.message === "string" ? candidate.message : parsed.message,
        };
      }
    }
  } catch {
    // Not our error shape — a proxy's HTML page, most likely. The status is
    // still the useful part.
  }

  throw errorFrom(response.status, parsed, response.headers.get("x-request-id"));
}

/** What to put in front of a person when one of the above fails. */
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

// ---------------------------------------------------------------------------
// Default precision — a local preference until the API carries one
// ---------------------------------------------------------------------------

const GRANULARITY_KEY = "pinvoyager.default_granularity";

/** `~1 km` rather than `exact`. Every voyage is world-readable, so the default
 *  has to be the cautious one: a user who never touches this control must not
 *  be publishing their exact coordinates because of it. */
const GRANULARITY_FALLBACK: Granularity = "km1";

function isGranularity(value: string | null): value is Granularity {
  return value !== null && (GRANULARITIES as readonly string[]).includes(value);
}

export function readDefaultGranularity(): Granularity {
  try {
    const stored = window.localStorage.getItem(GRANULARITY_KEY);
    return isGranularity(stored) ? stored : GRANULARITY_FALLBACK;
  } catch {
    // Storage disabled, or a private window that refuses it. The default is a
    // perfectly good answer.
    return GRANULARITY_FALLBACK;
  }
}

function writeDefaultGranularity(value: Granularity): void {
  try {
    window.localStorage.setItem(GRANULARITY_KEY, value);
  } catch {
    // Nothing to recover: the choice still applies for this session.
  }
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

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
          <EmptyState title="You are signed out" >
            Sign in again to reach your settings.
          </EmptyState>
        )}

        {user !== null && (
          <>
            <ProfileSection
              displayName={user.display_name}
              avatarUrl={user.avatar_url}
              onSaved={() => void refresh()}
            />
            <PrecisionSection />
            <ProvidersSection identities={identities} onChanged={() => void refresh()} />
            <DeleteSection />
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
  danger = false,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  danger?: boolean;
}) {
  return (
    <section
      className="pv-card"
      style={{
        padding: "var(--pv-space-5)",
        borderColor: danger ? "var(--pv-danger-bg)" : undefined,
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
    mutationFn: (name: string) => send("PATCH", "/api/me", { display_name: name }),
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
          Your picture comes from your sign-in provider. Initials are used when a
          provider does not supply one.
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
          help="Shown on your public profile and every voyage you publish."
          error={save.isError ? messageOf(save.error) : undefined}
        />

        <div style={{ marginTop: "var(--pv-space-4)" }}>
          <Button
            type="submit"
            variant="secondary"
            disabled={!dirty}
            loading={save.isPending}
          >
            Save name
          </Button>
          {save.isSuccess && !dirty && (
            <span className="pv-xs pv-muted" role="status" style={{ marginLeft: "var(--pv-space-3)" }}>
              Saved.
            </span>
          )}
        </div>
      </form>
    </Section>
  );
}

// ---------------------------------------------------------------------------
// Default precision
// ---------------------------------------------------------------------------

function PrecisionSection() {
  const [value, setValue] = useState<Granularity>(GRANULARITY_FALLBACK);
  const groupId = useId();

  // Read after mount rather than during render: `localStorage` is not available
  // in every rendering environment, and a lazy initialiser that touches it
  // makes the component unrenderable where it is not.
  useEffect(() => {
    setValue(readDefaultGranularity());
  }, []);

  return (
    <Section
      title="Default location precision"
      description="Applied to new voyages. Each voyage can override it."
    >
      <div
        className="pv-gran-steps"
        role="group"
        aria-labelledby={groupId}
        style={{ maxWidth: 340 }}
      >
        <span className="pv-sr" id={groupId}>
          Default location precision
        </span>
        {GRANULARITIES.map((step) => (
          <button
            key={step}
            type="button"
            className="pv-gran-step"
            aria-pressed={value === step}
            onClick={() => {
              setValue(step);
              writeDefaultGranularity(step);
            }}
          >
            {GRANULARITY_LABEL[step]}
          </button>
        ))}
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------------
// Sign-in methods
// ---------------------------------------------------------------------------

function ProvidersSection({
  identities,
  onChanged,
}: {
  identities: readonly LinkedProvider[];
  onChanged: () => void;
}) {
  const linked = new Map(identities.map((identity) => [identity.provider, identity]));

  // The whole point of this screen. One linked provider means unlinking it
  // locks the user out permanently, so the count — not a flag on a row — is
  // what decides whether any unlink control is live.
  const isLastProvider = linked.size <= 1;

  const unlink = useMutation({
    mutationFn: (provider: Provider) => send("DELETE", `/auth/link/${provider}`),
    retry: false,
    onSuccess: onChanged,
  });

  return (
    <Section
      title="Sign-in methods"
      description="Any linked provider signs you in to this same account. We store no passwords."
    >
      {linked.size === 0 && (
        <Toast tone="warning" title="No sign-in methods are listed">
          This should not be possible for a signed-in account. Reload the page
          before changing anything here.
        </Toast>
      )}

      <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
        {PROVIDERS.map((provider) => (
          <ProviderRow
            key={provider}
            provider={provider}
            identity={linked.get(provider)}
            isLastProvider={isLastProvider}
            busy={unlink.isPending && unlink.variables === provider}
            onUnlink={() => unlink.mutate(provider)}
          />
        ))}
      </ul>

      {isLastProvider && linked.size === 1 && (
        <div style={{ marginTop: "var(--pv-space-3)" }}>
          {/* The disabled control above is paired with this. A disabled control
              with no reason attached reads as a bug; this states the
              consequence and the remedy. */}
          <Toast tone="warning" title="This is your only way in">
            Link another provider before unlinking{" "}
            {PROVIDER_NAME[[...linked.keys()][0] ?? "google"]}, or you will lose
            access to this account permanently.
          </Toast>
        </div>
      )}

      {unlink.isError && (
        <div style={{ marginTop: "var(--pv-space-3)" }}>
          <Toast tone="error" title="That provider could not be unlinked">
            {messageOf(unlink.error)}
          </Toast>
        </div>
      )}

      <div
        className="pv-gran-guarantee"
        style={{
          marginTop: "var(--pv-space-4)",
          background: "var(--pv-accent-subtle)",
        }}
      >
        <WarningTriangleIcon size={16} stroke="var(--pv-accent)" />
        <div>
          <strong>Link a second provider.</strong> If you lose access to your only
          sign-in method, we cannot restore your account — there is no password to
          reset.
        </div>
      </div>
    </Section>
  );
}

function ProviderRow({
  provider,
  identity,
  isLastProvider,
  busy,
  onUnlink,
}: {
  provider: Provider;
  identity: LinkedProvider | undefined;
  isLastProvider: boolean;
  busy: boolean;
  onUnlink: () => void;
}) {
  const name = PROVIDER_NAME[provider];
  const reasonId = `unlink-${provider}-reason`;
  const soleProvider = identity !== undefined && isLastProvider;

  return (
    <li
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--pv-space-3)",
        padding: "var(--pv-space-3) 0",
        borderBottom: "1px solid var(--pv-border)",
        flexWrap: "wrap",
      }}
    >
      <div className="pv-upload-info">
        <div className="pv-sm" style={{ fontWeight: "var(--pv-weight-medium)" }}>
          {name}
        </div>
        <div className="pv-xs pv-subtle" id={soleProvider ? reasonId : undefined}>
          {identity === undefined
            ? "Not linked"
            : soleProvider
              ? "Your only sign-in method — it cannot be unlinked"
              : `Linked · last used ${relativeDay(identity.last_seen_at)}`}
        </div>
      </div>

      {identity === undefined ? (
        // A real form POST rather than a fetch: `/auth/link/{provider}`
        // answers 302 to the provider's consent screen, and a redirect chain
        // that ends off-origin has to be a navigation. POST rather than GET
        // because a `SameSite=Lax` cookie is not sent on a cross-site POST,
        // which is what stops another site starting a link flow in a signed-in
        // user's browser.
        <form
          method="post"
          action={`/auth/link/${provider}?redirect_to=%2Fsettings`}
          style={{ margin: 0 }}
        >
          <Button type="submit" variant="secondary">
            Link
          </Button>
        </form>
      ) : (
        <Button
          variant="ghost"
          disabled={soleProvider}
          loading={busy}
          aria-describedby={soleProvider ? reasonId : undefined}
          onClick={onUnlink}
        >
          Unlink
        </Button>
      )}
    </li>
  );
}

/** "2 days ago" from an ISO timestamp. Coarse on purpose — the exact minute a
 *  provider was last used is not information this screen is for. */
function relativeDay(iso: string): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "recently";

  const days = Math.floor((Date.now() - then) / 86_400_000);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 7) return `${String(days)} days ago`;
  if (days < 30) {
    const weeks = Math.floor(days / 7);
    return weeks === 1 ? "1 week ago" : `${String(weeks)} weeks ago`;
  }
  const months = Math.floor(days / 30);
  return months === 1 ? "1 month ago" : `${String(months)} months ago`;
}

// ---------------------------------------------------------------------------
// Deletion
// ---------------------------------------------------------------------------

function DeleteSection() {
  const [confirming, setConfirming] = useState(false);

  const destroy = useMutation({
    mutationFn: () => send("DELETE", "/api/me"),
    retry: false,
    onSuccess: () => {
      // Nothing left to render against: the account is gone, so the whole
      // application has to be re-entered from the signed-out state.
      window.location.assign("/");
    },
  });

  return (
    <Section
      title="Delete account"
      description="Permanently removes your account, your voyages, and your photos. Published voyages stop being reachable immediately — anyone who has linked to one will find a dead link. This cannot be undone."
      danger
    >
      {/* Two steps, and deliberately so: the first press only reveals the
          confirmation. A single click must never be able to destroy an account,
          and a `confirm()` dialogue is dismissed by reflex. */}
      {!confirming ? (
        <Button variant="danger" onClick={() => setConfirming(true)}>
          Delete my account
        </Button>
      ) : (
        <div className="pv-stack-3">
          <Toast tone="error" title="Delete this account permanently?">
            Your voyages and photos are removed and cannot be recovered. Links
            other people have to your published voyages will break.
          </Toast>

          <div className="pv-inline-3">
            <Button
              variant="danger"
              loading={destroy.isPending}
              onClick={() => destroy.mutate()}
            >
              Yes, delete my account permanently
            </Button>
            <Button
              variant="secondary"
              disabled={destroy.isPending}
              onClick={() => setConfirming(false)}
            >
              Keep my account
            </Button>
          </div>

          {destroy.isError && (
            <Toast tone="error" title="Your account was not deleted">
              {messageOf(destroy.error)}
            </Toast>
          )}
        </div>
      )}
    </Section>
  );
}
