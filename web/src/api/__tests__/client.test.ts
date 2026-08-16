/**
 * The three properties of the fetch wrapper that are invisible when they hold
 * and expensive when they do not.
 *
 * These are asserted against a stubbed `fetch` rather than a running service:
 * what is under test is what the client *sends* and what it *makes of* a
 * response, and both are decisions the client makes on its own.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ApiRequestError,
  UnauthorizedError,
  deleteVoyage,
  getVoyage,
  isUnauthorized,
  listVoyages,
  updateVoyage,
} from "../client";

/** A response factory rather than a response: a `Response` body can only be
 *  read once, so a stub that handed the same object to two calls would fail on
 *  the second for a reason that has nothing to do with the client. */
function respond(status: number, body: unknown, headers: Record<string, string> = {}) {
  return () => new Response(body === undefined ? null : JSON.stringify(body), { status, headers });
}

function stubFetch(make: () => Response) {
  const fetch = vi.fn(() => Promise.resolve(make()));
  vi.stubGlobal("fetch", fetch);
  return fetch;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the fetch wrapper", () => {
  /**
   * The session is an HttpOnly cookie. A request without this is anonymous, so
   * every authenticated call 401s and the application looks permanently signed
   * out — a failure that is easy to misdiagnose as broken sign-in.
   */
  it("always sends cookies", async () => {
    const fetch = stubFetch(respond(200, { voyages: [], next_before: null }));

    await listVoyages();

    const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(init.credentials).toBe("same-origin");
  });

  /**
   * For a 5xx the message is deliberately generic and the real cause is only in
   * the server log, retrievable by this header. The design's error screen asks
   * the user to quote it, so losing it makes the failure unreportable.
   */
  it("keeps the x-request-id from a failure", async () => {
    stubFetch(
      respond(
        500,
        { code: "internal", message: "An unexpected error occurred." },
        { "x-request-id": "01a00ab0-64a9-72a1-8627-367f1109210c" },
      ),
    );

    const error = await getVoyage("some-id").catch((err: unknown) => err);

    expect(error).toBeInstanceOf(ApiRequestError);
    const api = error as ApiRequestError;
    expect(api.status).toBe(500);
    expect(api.code).toBe("internal");
    expect(api.message).toBe("An unexpected error occurred.");
    expect(api.requestId).toBe("01a00ab0-64a9-72a1-8627-367f1109210c");
  });

  /** "Sign in" and "something broke" are different screens. */
  it("makes a 401 distinguishable from every other failure", async () => {
    stubFetch(respond(401, { code: "unauthorized", message: "Authentication is required." }));

    const error = await getVoyage("some-id").catch((err: unknown) => err);

    expect(error).toBeInstanceOf(UnauthorizedError);
    expect(isUnauthorized(error)).toBe(true);

    stubFetch(respond(404, { code: "not_found", message: "The requested resource was not found." }));
    const missing = await getVoyage("some-id").catch((err: unknown) => err);
    expect(missing).toBeInstanceOf(ApiRequestError);
    expect(isUnauthorized(missing)).toBe(false);
  });

  /** A 422 names the field, so the edit form can put the message beside the
   *  input rather than in a banner. */
  it("carries field errors off a 422", async () => {
    stubFetch(
      respond(422, {
        code: "validation_failed",
        message: "A title is required before publishing.",
        errors: [{ field: "title", message: "A title is required before publishing." }],
      }),
    );

    const error = (await updateVoyage("id", { published: true }).catch(
      (err: unknown) => err,
    )) as ApiRequestError;

    expect(error.fieldError("title")).toBe("A title is required before publishing.");
    expect(error.fieldError("description")).toBeUndefined();
  });

  /** `response.json()` on an empty body throws, and the throw would be reported
   *  as a failed delete that in fact succeeded. */
  it("reads a 204 as no body rather than as broken JSON", async () => {
    stubFetch(() => new Response(null, { status: 204 }));
    await expect(deleteVoyage("id")).resolves.toBeUndefined();
  });

  /** An absent parameter and an empty one are different requests: `limit=` is a
   *  parse failure server-side. */
  it("omits query parameters that were not given", async () => {
    const fetch = stubFetch(respond(200, { voyages: [], next_before: null }));

    await listVoyages({ limit: 20 });

    const [url] = fetch.mock.calls[0] as unknown as [string];
    expect(url).toBe("/api/voyages?limit=20");
  });

  /** `mine=false` is the same request as no `mine` at all, and sending it would
   *  make two identical reads look like two different ones. */
  it("does not send mine=false", async () => {
    const fetch = stubFetch(respond(200, { voyages: [], next_before: null }));

    await listVoyages({ mine: false });

    const [url] = fetch.mock.calls[0] as unknown as [string];
    expect(url).toBe("/api/voyages");
  });

  /**
   * Three states, not two. An omitted description is left alone and a `null`
   * one is cleared, and a PATCH that could not tell them apart would make
   * "remove my description" impossible to express.
   */
  it("distinguishes an absent description from an explicit null", async () => {
    const fetch = stubFetch(respond(200, {}));

    await updateVoyage("id", { title: "Hokkaido" });
    let [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({ title: "Hokkaido" });

    await updateVoyage("id", { description: null });
    [, init] = fetch.mock.calls[1] as unknown as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({ description: null });
  });

  /** A proxy in front of the service can answer with an HTML error page. The
   *  status is still the useful part, and a JSON parse failure must not become
   *  the error the user sees. */
  it("survives an error body that is not JSON", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response("<html>502</html>", { status: 502 }))),
    );

    const error = (await getVoyage("id").catch((err: unknown) => err)) as ApiRequestError;

    expect(error).toBeInstanceOf(ApiRequestError);
    expect(error.status).toBe(502);
    expect(error.code).toBe("unknown");
  });
});
