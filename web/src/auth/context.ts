/**
 * The shape of the session as the rest of the application sees it.
 *
 * Split out from `AuthProvider.tsx` so that file exports only a component:
 * mixing a context object into a module that also exports components defeats
 * fast refresh, and the lint rule that catches it is there for that reason.
 */

import { createContext } from "react";

import type { CurrentUser, LinkedProvider } from "../api/types";

/**
 * Three states, not two.
 *
 * `loading` is separate from `anonymous` on purpose. Collapsing them makes the
 * first paint of every protected route a redirect to sign-in, because "we have
 * not asked yet" and "we asked and nobody is signed in" would be the same
 * value — and a signed-in user reloading `/upload` would be bounced to the
 * sign-in screen and then back, which looks like the session was lost.
 *
 * `failed` is separate from `anonymous` for the mirror-image reason: a 500 or a
 * dropped connection is not evidence that the visitor has no account.
 */
export type AuthStatus = "loading" | "anonymous" | "authenticated" | "failed";

export interface AuthState {
  status: AuthStatus;
  /** Non-null exactly when `status` is `authenticated`. */
  user: CurrentUser | null;
  /** The providers linked to this account. Empty unless authenticated. */
  identities: LinkedProvider[];
  /** Set exactly when `status` is `failed`. A 401 never lands here. */
  error: Error | null;
  /** Re-reads `/api/me`. Awaited by callers that need the answer, such as the
   *  callback screen deciding whether sign-in actually worked. */
  refresh: () => Promise<void>;
  /** Destroys the session server-side and drops to the anonymous state. */
  signOut: () => Promise<void>;
}

/**
 * `null` rather than a default state, so `useAuth` can tell "outside the
 * provider" from "inside it, still loading" and say so rather than silently
 * reporting every consumer as signed out.
 */
export const AuthContext = createContext<AuthState | null>(null);
