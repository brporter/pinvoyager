/** Reads the session state established by `AuthProvider`. */

import { useContext } from "react";

import { AuthContext } from "./context";
import type { AuthState } from "./context";

/**
 * Throws outside an `AuthProvider` rather than returning a signed-out default.
 *
 * A default would turn "this subtree was rendered outside the provider" into
 * "this user is not signed in", which is a bug that looks exactly like a
 * correctly working anonymous session and is therefore invisible until someone
 * reports that sign-in does not stick on one screen.
 */
export function useAuth(): AuthState {
  const state = useContext(AuthContext);

  if (state === null) {
    throw new Error("useAuth must be used inside an <AuthProvider>");
  }

  return state;
}
