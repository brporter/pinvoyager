/**
 * The shape of the session as the rest of the application sees it.
 *
 * Split out from `AuthProvider.tsx` so that file exports only a component:
 * mixing a context object into a module that also exports components defeats
 * fast refresh, and the lint rule that catches it is there for that reason.
 */

import { createContext } from "react";

import type { CurrentUser, LinkedProvider } from "../api/types";

/** Loading and failed reads do not imply that the visitor is signed out. */
type SessionState =
  | { status: "loading"; user: null; identities: LinkedProvider[]; error: null }
  | { status: "anonymous"; user: null; identities: LinkedProvider[]; error: null }
  | { status: "authenticated"; user: CurrentUser; identities: LinkedProvider[]; error: null }
  | { status: "failed"; user: null; identities: LinkedProvider[]; error: Error };
export type AuthStatus = SessionState["status"];
export type AuthState = SessionState & {
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
};

/**
 * `null` rather than a default state, so `useAuth` can tell "outside the
 * provider" from "inside it, still loading" and say so rather than silently
 * reporting every consumer as signed out.
 */
export const AuthContext = createContext<AuthState | null>(null);
