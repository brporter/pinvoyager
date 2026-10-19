/** Complete profile setup after the provider establishes a session. */
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { updateDisplayName } from "../api/client";
import { Button, TextField } from "../components";
import { Brand } from "../components/Brand";
import { AuthCard } from "./AuthCard";
import { useAuth } from "./useAuth";

export function ChooseDisplayName() {
  const [name, setName] = useState("");
  const { refresh } = useAuth();
  const save = useMutation({
    mutationFn: updateDisplayName,
    onSuccess: () => refresh(),
    retry: false,
  });
  return (
    <>
      <nav className="pv-topnav" aria-label="Main">
        <Brand />
      </nav>
      <div
        className="pv-container"
        style={{ display: "flex", justifyContent: "center", paddingBlock: "var(--pv-space-8)" }}
      >
        <AuthCard labelledBy="choose-name-heading">
          <h1 className="pv-h3" id="choose-name-heading">
            Choose your display name
          </h1>
          <p className="pv-sm pv-muted">Your unique name also creates your public voyage URL.</p>
          <form
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              if (name.trim() !== "") save.mutate(name);
            }}
          >
            <TextField
              label="Display name"
              required
              autoComplete="username"
              maxLength={30}
              value={name}
              onChange={(event) => setName(event.target.value)}
              help="3–30 letters, digits, or hyphens. Start with a letter and end with a letter or digit."
              error={save.isError ? save.error.message : undefined}
            />
            <p className="pv-xs pv-muted">Public profile: /u/{name.trim().toLowerCase()}</p>
            <Button
              type="submit"
              variant="primary"
              loading={save.isPending}
              disabled={save.isPending || name.trim() === ""}
            >
              Continue
            </Button>
          </form>
        </AuthCard>
      </div>
    </>
  );
}
