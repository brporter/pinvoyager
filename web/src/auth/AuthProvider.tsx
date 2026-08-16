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
 * been visible only to that session — the caller's drafts, their library, the
 * exact coordinates in it — and a cache that survived sign-out would leave the
 * previous user's data on screen for the next one.
 */

import { useCallback, useMemo } from "react";
import type { ReactNode } from "react";

import { useMe, useSignOut } from "../api/hooks";
import { AuthContext } from "./context";
import type { AuthState, AuthStatus } from "./context";

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
    // Order matters. `isPending` first, because a query that has never
    // resolved has `data === undefined` and would otherwise be read as
    // anonymous — which is how every protected route ends up redirecting to
    // sign-in for a moment on reload.
    const status: AuthStatus = me.isPending
      ? "loading"
      : me.isError
        ? "failed"
        : me.data === null
          ? "anonymous"
          : "authenticated";

    return {
      status,
      user: me.data?.user ?? null,
      identities: me.data?.identities ?? [],
      // A 401 never reaches here: `useMe` resolves it to `null`, because "not
      // signed in" is an ordinary answer in an application that is almost
      // entirely readable without an account. Only a real failure — a 5xx, a
      // dropped connection — sets this.
      error: me.isError ? me.error : null,
      refresh,
      signOut,
    };
  }, [me.isPending, me.isError, me.data, me.error, refresh, signOut]);

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}
