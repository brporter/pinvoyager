/**
 * Shared scaffolding for the screen tests.
 *
 * The screens read their data through `src/api/hooks.ts` and the session
 * through `useAuth`. Both are supplied here rather than by standing a server
 * up: what these tests are about is what a screen does with an answer, and a
 * real backend makes "what happens when this 503s" harder to ask, not easier.
 *
 * `renderScreen` injects the auth context directly instead of wrapping in
 * `AuthProvider`, because a suite that mocks the hooks module has no `useMe`
 * for the provider to call.
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { InfiniteData, UseInfiniteQueryResult } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { RenderResult } from "@testing-library/react";
import type { ReactElement } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { vi } from "vitest";

import { GRANULARITY_LABEL, GRANULARITY_PRECISION } from "../../components/granularity";
import { ApiRequestError } from "../../api/client";
import type { CurrentUser, VoyageList, VoyageSummary } from "../../api/types";
import { AuthContext } from "../../auth/context";
import type { AuthState, AuthStatus } from "../../auth/context";

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

export const ADA: CurrentUser = {
  id: "0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
  display_name: "ada-lovelace",
  avatar_url: null,
  created_at: "2026-02-01T10:00:00Z",
};

/** Somebody else's id. Used for the voyages a profile must not claim, and for
 *  the creator whose name is not on the wire at all. */
export const STRANGER_ID = "0192f0c2-9999-7c3d-8e4f-5a6b7c8d9e0f";

/**
 * The session as the shell would have established it.
 *
 * `status` is derived from `user` unless it is given, so the common cases read
 * as `authState(ADA)` and `authState(null)` and only the awkward ones —
 * "still loading", "the read failed" — have to say so.
 */
export function authState(user: CurrentUser): Extract<AuthState, { status: "authenticated" }>;
export function authState(user: null): Extract<AuthState, { status: "anonymous" }>;
export function authState<S extends AuthStatus>(
  user: null,
  status: S,
): Extract<AuthState, { status: S }>;
export function authState(
  user: CurrentUser | null,
  status: AuthStatus = user === null ? "anonymous" : "authenticated",
): AuthState {
  const actions = { refresh: () => Promise.resolve(), signOut: () => Promise.resolve() };
  if (status === "authenticated" && user !== null)
    return { ...actions, status, user, identities: [], error: null };
  if (status === "failed")
    return {
      ...actions,
      status,
      user: null,
      identities: [],
      error: new Error("fixture read failed"),
    };
  if (status === "authenticated") throw new Error("authenticated fixtures require a user");
  return { ...actions, status, user: null, identities: [], error: null };
}

// ---------------------------------------------------------------------------
// Voyages
// ---------------------------------------------------------------------------

export function voyage(overrides: Partial<VoyageSummary> = {}): VoyageSummary {
  return {
    cover_url: null,
    photos_pending: false,
    id: "0192f0c2-aaaa-7c3d-8e4f-5a6b7c8d9e0f",
    owner_id: ADA.id,
    title: "Three weeks in Hokkaido",
    description: "Ferries, snow, and a great deal of ramen.",
    published: true,
    granularity: "km1",
    created_at: "2026-08-01T10:00:00Z",
    published_at: "2026-08-02T10:00:00Z",
    ...overrides,
    granularity_label: GRANULARITY_LABEL[overrides.granularity ?? "km1"],
    precision_m: GRANULARITY_PRECISION[overrides.granularity ?? "km1"],
  };
}

/** An unpublished voyage. Only ever returned by `mine=true`, and only ever
 *  shown to its owner. */
export function draft(overrides: Partial<VoyageSummary> = {}): VoyageSummary {
  return voyage({
    id: "0192f0c2-bbbb-7c3d-8e4f-5a6b7c8d9e0f",
    title: "Sanriku coast, unfinished",
    published: false,
    published_at: null,
    ...overrides,
  });
}

type Feed = UseInfiniteQueryResult<InfiniteData<VoyageList, string | null>>;

/**
 * A `useVoyages` result with only the members the screens read.
 *
 * Cast rather than constructed in full: React Query's result type has some
 * thirty fields, and writing all of them would be scaffolding pretending to be
 * a query. A screen that starts reading a field this omits fails loudly with
 * `undefined` rather than passing against a fabricated value.
 */
export function feedResult(overrides: Partial<Feed> = {}): Feed {
  return {
    data: undefined,
    isPending: false,
    isError: false,
    error: null,
    hasNextPage: false,
    isFetchingNextPage: false,
    fetchNextPage: vi.fn(),
    refetch: vi.fn(),
    ...overrides,
  } as unknown as Feed;
}

/** One page of voyages. Passing `nextBefore` is what makes the feed look
 *  unfinished — which several screens treat as "do not claim a total yet". */
export function loadedFeed(voyages: VoyageSummary[], nextBefore?: string): Feed {
  return feedResult({
    data: {
      pages: [{ voyages, next_before: nextBefore ?? null }],
      pageParams: [null],
    },
    hasNextPage: nextBefore !== undefined,
  });
}

/** A failed read, carrying the `x-request-id` the error screens quote. */
export function apiFailure(status = 500, requestId: string | null = null): Feed {
  return feedResult({
    isError: true,
    error: new ApiRequestError({
      status,
      code: status >= 500 ? "internal" : "bad_request",
      message: "Something went wrong.",
      requestId,
    }),
  });
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

export interface RenderScreenOptions {
  /** The address to render at. Only meaningful together with `route`. */
  path?: string;
  /** The route pattern, when the screen reads `useParams`. */
  route?: string;
  auth?: AuthState;
}

export function renderScreen(ui: ReactElement, options: RenderScreenOptions = {}): RenderResult {
  const { path = "/", route, auth = authState(null) } = options;

  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, retryDelay: 0 },
      mutations: { retry: false },
    },
  });

  return render(
    <QueryClientProvider client={client}>
      <AuthContext.Provider value={auth}>
        <MemoryRouter initialEntries={[path]}>
          {route === undefined ? (
            ui
          ) : (
            <Routes>
              <Route path={route} element={ui} />
            </Routes>
          )}
        </MemoryRouter>
      </AuthContext.Provider>
    </QueryClientProvider>,
  );
}
