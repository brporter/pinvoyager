/**
 * Forged sign-in, for development only.
 *
 * # This is an authentication bypass
 *
 * It posts a name to `POST /auth/dev/login` and receives a session for it. No
 * provider, no `state`, no PKCE, no ID token, no signature. It exists because
 * obtaining credentials for four providers is a serialised, human-gated process
 * — Apple alone needs a paid developer account — and without it nobody can sign
 * in on a development machine at all, which makes every signed-in screen in
 * this application unbuildable and untestable.
 *
 * ## How it stays out of production
 *
 * Three independent layers, in order of how much they are worth relying on:
 *
 *  1. **The server route does not exist in a release build.** Both the handler
 *     and its route are behind `#[cfg(debug_assertions)]`, which Cargo turns
 *     off in the release profile, and the container image builds `--release`.
 *     Absent, not disabled. This is the guarantee that actually matters, and it
 *     does not depend on anything in this file.
 *  2. **The server refuses unless `auth.dev_login` is set**, answering 404 —
 *     indistinguishable from a bypass that was never built.
 *  3. **This component is not in the production bundle.** `SignIn` renders it
 *     behind `import.meta.env.DEV`, which Vite replaces with the literal
 *     `false` at build time; the branch folds away, the import becomes
 *     unreferenced, and Rollup drops the module. `npm run build` output can be
 *     grepped for the strings below to confirm it, and that check is worth
 *     repeating whenever the bundler configuration changes.
 *
 * Layer 3 is a bundle-size and honesty measure. It is not a security control —
 * a UI that is absent cannot stop a request that layer 1 has already made
 * impossible to serve.
 */

import { useState } from "react";

import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { TextField } from "../components/TextField";

export function DevSignIn({ onSignedIn }: { onSignedIn: () => void | Promise<void> }) {
  const [subject, setSubject] = useState("dev@example.com");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Not routed through `src/api/client`. That module is the typed surface of
  // the real API, and this endpoint is not part of it — it does not exist in a
  // release build, so a client method for it would be a permanent reference to
  // something that is usually absent.
  const submit = async () => {
    setBusy(true);
    setError(null);

    try {
      const response = await fetch("/auth/dev/login", {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ subject }),
      });

      if (response.status === 404) {
        setError(
          "The development sign-in is not available. It needs a debug build of the API with auth.dev_login enabled.",
        );
        return;
      }

      if (!response.ok) {
        setError(`Development sign-in failed (${String(response.status)}).`);
        return;
      }

      await onSignedIn();
    } catch {
      setError("Could not reach the API. Is it running on :8080?");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      style={{
        marginTop: "var(--pv-space-6)",
        paddingTop: "var(--pv-space-5)",
        borderTop: "1px dashed var(--pv-border-strong)",
      }}
    >
      <div style={{ marginBottom: "var(--pv-space-3)" }}>
        <Badge tone="draft">Development only</Badge>
      </div>

      <form
        className="pv-stack-3"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <TextField
          label="Sign in as"
          name="subject"
          type="text"
          autoComplete="off"
          value={subject}
          onChange={(event) => {
            setSubject(event.target.value);
          }}
          help="Stored as an opaque provider subject. It looks like an email because that is a convenient thing to type — nothing parses it and nothing is ever sent to it."
          error={error ?? undefined}
        />

        <Button
          variant="secondary"
          type="submit"
          size="lg"
          loading={busy}
          disabled={subject.trim() === ""}
          style={{ width: "100%" }}
        >
          Forge a session
        </Button>
      </form>
    </div>
  );
}
