/** Shared session fixtures, fetch stubs, and render helpers for auth tests. */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import type { ReactNode } from "react";
import { useLocation } from "react-router";
import { vi } from "vitest";

import type { Me } from "../../api/types";

export const SIGNED_IN: Me = {
  user: {
    id: "0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
    display_name: "ada-lovelace",
    avatar_url: null,
    created_at: "2026-02-01T10:00:00Z",
  },
  identities: [
    { provider: "google", linked_at: "2026-02-01T10:00:00Z", last_seen_at: "2026-08-16T09:00:00Z" },
  ],
};

/**
 * A response object with only the members the client touches.
 *
 * Built by hand rather than with `new Response(...)` so the tests do not depend
 * on which fetch implementation the environment happens to provide.
 */
function stubResponse(status: number, body: unknown): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: "",
    headers: new Headers({ "content-type": "application/json" }),
    text: () => Promise.resolve(body === undefined ? "" : JSON.stringify(body)),
  } as unknown as Response;
}

/**
 * Installs a `fetch` that answers `GET /api/me` with `status`, and leaves every
 * other call to throw — an unexpected request should fail a test rather than
 * silently resolve to undefined.
 */
export function stubMe(status: number, body?: unknown): void {
  const payload =
    body ??
    (status === 401
      ? { code: "unauthorized", message: "Not signed in." }
      : { code: "internal", message: "Something went wrong." });

  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : String(input);

      if (url === "/api/me") {
        return Promise.resolve(stubResponse(status, payload));
      }

      return Promise.reject(new Error(`unexpected fetch to ${url}`));
    }),
  );
}

/**
 * Creates a query client with zero retry delay. Hook-specific retry rules
 * still apply, so tests can reach settled error states without waiting for backoff.
 */
export function testQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retryDelay: 0 },
      mutations: { retry: false },
    },
  });
}

export function WithQueryClient({ children }: { children: ReactNode }) {
  // Lazy, and held in state: building it inline would mint a fresh cache on
  // every render and quietly discard whatever the last one fetched.
  const [client] = useState(testQueryClient);
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/** Renders the current location so a test can assert on a redirect's target
 *  and not merely on what ended up on screen. */
export function LocationProbe() {
  const location = useLocation();
  return (
    <span data-testid="location">
      {location.pathname}
      {location.search}
    </span>
  );
}
