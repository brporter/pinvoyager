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
  NetworkError,
  UnauthorizedError,
  createVoyage,
  deleteVoyage,
  deletePhoto,
  detachPhoto,
  errorFrom,
  getDiscoverMap,
  getMe,
  getPublicProfile,
  updateDisplayName,
  getPhoto,
  getVoyage,
  getVoyageMap,
  isUnauthorized,
  listPhotos,
  listVoyages,
  reorderPhotos,
  signInUrl,
  signOut,
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
  it("saves names through authenticated PATCH and encodes public profile paths and pagination", async () => {
    const save = stubFetch(respond(204, undefined));
    await updateDisplayName("betaporter");
    expect(save).toHaveBeenCalledWith(
      "/api/me",
      expect.objectContaining({
        method: "PATCH",
        credentials: "same-origin",
        body: JSON.stringify({ display_name: "betaporter" }),
      }),
    );
    const fetch = stubFetch(respond(200, { user: {}, voyages: [], next_before: null }));
    await getPublicProfile("betaporter");
    expect(fetch).toHaveBeenLastCalledWith("/api/profiles/betaporter", expect.anything());
    const controller = new AbortController();
    await getPublicProfile("a/b", { before: "cursor", limit: 2, signal: controller.signal });
    expect(fetch).toHaveBeenLastCalledWith(
      "/api/profiles/a%2Fb?before=cursor&limit=2",
      expect.objectContaining({ signal: controller.signal }),
    );
  });

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

    stubFetch(
      respond(404, { code: "not_found", message: "The requested resource was not found." }),
    );
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

  it("uses generic fields for an empty or structurally invalid error body", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response(null, { status: 503 }))),
    );
    const empty = (await getMe().catch((caught: unknown) => caught)) as ApiRequestError;
    expect(empty.message).toBe("The request failed.");

    stubFetch(respond(400, { code: 12, message: false, errors: "not an array" }));
    const invalid = (await getMe().catch((caught: unknown) => caught)) as ApiRequestError;
    expect(invalid.code).toBe("unknown");
    expect(invalid.message).toBe("The request failed.");
    expect(invalid.fieldErrors).toEqual([]);

    stubFetch(respond(400, "a primitive error value"));
    const primitive = (await getMe().catch((caught: unknown) => caught)) as ApiRequestError;
    expect(primitive.code).toBe("unknown");

    stubFetch(respond(400, null));
    const nullBody = (await getMe().catch((caught: unknown) => caught)) as ApiRequestError;
    expect(nullBody.code).toBe("unknown");
    expect(nullBody.message).toBe("The request failed.");
  });

  it("wraps fetch rejection as a network error and passes an abort signal", async () => {
    const cause = new TypeError("connection reset");
    const fetch = vi.fn().mockRejectedValue(cause);
    vi.stubGlobal("fetch", fetch);
    const controller = new AbortController();

    const error = await getMe({ signal: controller.signal }).catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(NetworkError);
    expect((error as NetworkError).cause).toBe(cause);
    expect((fetch.mock.calls[0] as unknown as [string, RequestInit])[1].signal).toBe(
      controller.signal,
    );
  });

  it.each([204, 205])("treats HTTP %i as a successful empty response", async (status) => {
    stubFetch(() => new Response(null, { status }));
    await expect(getMe()).resolves.toBeUndefined();
  });

  it("returns undefined for a successful response with an empty body", async () => {
    stubFetch(() => new Response("", { status: 200 }));
    await expect(getMe()).resolves.toBeUndefined();
  });

  it("propagates malformed JSON in a successful response", async () => {
    stubFetch(() => new Response("{", { status: 200 }));
    await expect(getMe()).rejects.toBeInstanceOf(SyntaxError);
  });

  it("uses fallback error fields when the response body cannot be read or parsed", async () => {
    const unreadable = {
      ok: false,
      status: 503,
      statusText: "Unavailable",
      headers: new Headers(),
      text: vi.fn().mockRejectedValue(new Error("stream reset")),
    } as unknown as Response;
    vi.stubGlobal("fetch", vi.fn().mockResolvedValueOnce(unreadable));

    const readError = await getMe().catch((caught: unknown) => caught);
    expect(readError).toBeInstanceOf(ApiRequestError);
    expect((readError as ApiRequestError).message).toBe("Unavailable");

    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response("null", { status: 500 }))),
    );
    const parsedNull = await getMe().catch((caught: unknown) => caught);
    expect((parsedNull as ApiRequestError).code).toBe("unknown");
  });

  it("parses numeric Retry-After values and ignores HTTP dates", async () => {
    stubFetch(
      respond(429, { code: "rate_limited", message: "Slow down." }, { "retry-after": "12" }),
    );
    const rateLimit = (await getMe().catch((caught: unknown) => caught)) as ApiRequestError;
    expect(rateLimit.retryAfterSeconds).toBe(12);

    stubFetch(
      respond(
        429,
        { code: "rate_limited", message: "Slow down." },
        {
          "retry-after": "Wed, 21 Oct 2030 07:28:00 GMT",
        },
      ),
    );
    const date = (await getMe().catch((caught: unknown) => caught)) as ApiRequestError;
    expect(date.retryAfterSeconds).toBeNull();
  });
});

describe("endpoint wrappers", () => {
  it("sends the correct paths, methods, bodies, and query values", async () => {
    const fetch = stubFetch(() => new Response(null, { status: 204 }));
    const call = async (invoke: () => Promise<unknown>) => {
      const before = fetch.mock.calls.length;
      await invoke();
      return fetch.mock.calls[before] as unknown as [string, RequestInit];
    };

    expect((await call(() => getVoyageMap("v1")))[0]).toBe("/api/voyages/v1/map");
    expect((await call(() => listVoyages({ mine: true })))[0]).toBe("/api/voyages?mine=true");
    expect(
      (await call(() => getDiscoverMap({ west: -1, south: -2, east: 3, north: 4, zoom: 5 })))[0],
    ).toBe("/api/map?west=-1&south=-2&east=3&north=4&zoom=5");
    const create = await call(() => createVoyage({ title: "Trip" }));
    expect(create[0]).toBe("/api/voyages");
    expect(create[1].method).toBe("POST");
    expect(JSON.parse(create[1].body as string)).toEqual({ title: "Trip" });

    const reorder = await call(() => reorderPhotos("v1", ["p2", "p1"]));
    expect(reorder[0]).toBe("/api/voyages/v1/photos/order");
    expect(reorder[1].method).toBe("PATCH");

    expect((await call(() => detachPhoto("v1", "p1")))[0]).toBe("/api/voyages/v1/photos/p1");
    expect((await call(() => listPhotos("v1", { before: "cursor", limit: 7 })))[0]).toBe(
      "/api/voyages/v1/photos?before=cursor&limit=7",
    );
    expect((await call(() => getPhoto("p1")))[0]).toBe("/api/photos/p1");
    expect((await call(() => deletePhoto("p1")))[0]).toBe("/api/photos/p1");
    expect((await call(() => signOut()))[0]).toBe("/auth/logout");
  });

  it("constructs provider sign-in URLs with an optional encoded destination", () => {
    expect(signInUrl("google")).toBe("/auth/google/start");
    expect(signInUrl("apple", "/voyages/a b")).toBe(
      "/auth/apple/start?redirect_to=%2Fvoyages%2Fa+b",
    );
  });

  it("normalizes error classes and empty field-error defaults", () => {
    const unauthorized = errorFrom(401, { code: "unauthorized", message: "Sign in." }, null);
    expect(unauthorized).toBeInstanceOf(UnauthorizedError);
    expect(unauthorized.fieldError("title")).toBeUndefined();
    const ordinary = errorFrom(400, { code: "bad_request", message: "Bad." }, null);
    expect(ordinary.name).toBe("ApiRequestError");
    expect(ordinary.retryAfterSeconds).toBeNull();
  });
});

it("sends a reviewed strong content revision and preserves a 412 conflict", async () => {
  stubFetch(respond(412, { code: "revision_conflict", message: "Review the current version." }));
  await expect(
    updateVoyage("v1", { title: "Local edits" }, { revision: "17" }),
  ).rejects.toMatchObject({ status: 412, code: "revision_conflict" });
  expect(new Headers(vi.mocked(globalThis.fetch).mock.calls[0][1]?.headers).get("if-match")).toBe(
    '"v:17"',
  );
});
