/** Reads the session state established by `AuthProvider`. */

import { useContext } from "react";

import { AuthContext } from "./context";
import type { AuthState } from "./context";

/** Returns session context. Throws when called outside AuthProvider. */
export function useAuth(): AuthState {
  const state = useContext(AuthContext);

  if (state === null) {
    throw new Error("useAuth must be used inside an <AuthProvider>");
  }

  return state;
}
