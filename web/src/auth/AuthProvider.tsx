/**
 * Establishes the current-user state for the whole application (T5).
 *
 * Thin on purpose. The read itself is `useMe` in `src/api/hooks.ts` — one
 * query, one cache entry, one retry policy — and this component's job is to
 * turn that query's four booleans into the four states the interface actually
 * branches on, and to hold them at the root so the answer is fetched once
 * rather than once per screen that wonders.
 *
 * Sign-out goes through `useSignOut` for a reason that is not obvious: it
 * empties the query cache afterwards. Anything fetched while signed in may have
 * been visible only to that session — the caller's drafts, their owner-only photos, the
 * exact coordinates in it — and a cache that survived sign-out would leave the
 * previous user's data on screen for the next one.
 */

import { useCallback, useMemo } from "react";
import type { ReactNode } from "react";

import { useMe, useSignOut } from "../api/hooks";
import { ChooseDisplayName } from "./ChooseDisplayName";
import { AuthContext } from "./context";
import type { AuthState } from "./context";

export function AuthProvider({ children }: { children: ReactNode }) {
  const me = useMe();
  const signOutMutation = useSignOut();

  const { mutateAsync } = signOutMutation;
  const { refetch } = me;

  const refresh = useCallback(async (): Promise<void> => {
    await refetch();
  }, [refetch]);

  const signOut = useCallback(async (): Promise<void> => {
    try {
      await mutateAsync();
    } catch {
      // The endpoint answers 204 even when there was no session, so a failure
      // here is the network rather than a refusal. Raising it would leave the
      // interface claiming the user is still signed in while the cookie may
      // well be gone; the next `/api/me` settles it either way.
    }
  }, [mutateAsync]);

  const value = useMemo<AuthState>(() => {
    const actions = { refresh, signOut };
    if (me.isPending)
      return { ...actions, status: "loading", user: null, identities: [], error: null };
    if (me.isError)
      return { ...actions, status: "failed", user: null, identities: [], error: me.error };
    if (me.data == null)
      return { ...actions, status: "anonymous", user: null, identities: [], error: null };
    return {
      ...actions,
      status: "authenticated",
      user: me.data.user,
      identities: me.data.identities,
      error: null,
    };
  }, [me.isPending, me.isError, me.data, me.error, refresh, signOut]);

  return (
    <AuthContext.Provider value={value}>
      {value.status === "authenticated" &&
      value.user !== null &&
      value.user.display_name === null ? (
        <ChooseDisplayName />
      ) : (
        children
      )}
    </AuthContext.Provider>
  );
}
