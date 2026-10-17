/**
 * Development sign-in form that requests a session for a supplied subject
 * without contacting a provider. Rendered only in development. The server
 * additionally requires debug assertions and auth.dev_login; otherwise the
 * endpoint returns 404.
 */

import { useState } from "react";

import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { TextField } from "../components/TextField";

export function DevSignIn({ onSignedIn }: { onSignedIn: () => void | Promise<void> }) {
  const [subject, setSubject] = useState("dev@example.com");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Call the development-only endpoint directly; it is absent in release builds.
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
