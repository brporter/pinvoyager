/**
 * Maps the shared current-user query to loading, failed, anonymous, or
 * authenticated session state. Successful sign-out clears query data; failed
 * sign-out requests propagate so callers can offer retry.
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
    await mutateAsync();
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
