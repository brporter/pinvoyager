/**
 * The HTTP client for the PinVoyager API.
 *
 * Every call in this module goes through {@link request}, which is where the
 * three things that are easy to get wrong once and never notice again live:
 *
 *  1. **Cookies are always sent.** The session is an `HttpOnly` cookie set by
 *     the service, so `credentials: "same-origin"` is not a nicety — without it
 *     every authenticated request is anonymous and the whole app looks signed
 *     out. It is set here rather than at each call site precisely so that a new
 *     endpoint cannot forget it.
 *  2. **A failure keeps its `x-request-id`.** For a 5xx the response body says
 *     nothing useful on purpose; the real cause is in the server log, findable
 *     only by that header. The design's error screen asks the user to quote it,
 *     so an error object that dropped it would make the failure unreportable.
 *  3. **A 401 is a different kind of thing.** It means "sign in", not
 *     "something broke", and the two want different screens. It arrives as
 *     {@link UnauthorizedError}, a subclass, so callers can branch with
 *     `instanceof` rather than comparing numbers.
 *
 * Response shapes come from `./types`, which is checked against fixtures
 * captured from the running service — see `__tests__/contract.test.ts`.
 */

import type {
  DiscoverMap,
  Granularity,
  Me,
  Photo,
  PhotoList,
  UploadResult,
  Voyage,
  VoyageList,
  VoyageMap,
} from "./types";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/** One field-level complaint from a 422. Rust: the `errors` array that
 *  `http::voyages::Rejection::Field` adds beside `code` and `message`. */
export interface FieldError {
  field: string;
  message: string;
}

/**
 * An error body as it arrives on the wire.
 *
 * Deliberately wider than `ApiError` in `./types`. That type lists the seven
 * codes `AppError` produces; the two handler-local `Rejection` types add
 * `validation_failed`, `too_large`, `not_an_image`, `unprocessable_image`,
 * `rate_limited` and `variants_pending`, and the first of those also carries an
 * `errors` array. `code` is typed as `string` here so that a code this build
 * has never heard of still parses — a client that threw on an unknown code
 * would turn a new server-side rejection into a blank screen.
 */
export interface ApiErrorBody {
  code: string;
  message: string;
  errors?: FieldError[];
}

/**
 * Any non-2xx response.
 *
 * Carries the status, the stable `code`, the human `message` and the request
 * id. Branch on `code`; render `message`; quote `requestId`.
 */
export class ApiRequestError extends Error {
  readonly status: number;
  readonly code: string;
  /** The `x-request-id` response header, or null if the response carried none
   *  (a network failure, or a proxy that stripped it). */
  readonly requestId: string | null;
  /** Populated on a 422. Empty for every other status. */
  readonly fieldErrors: readonly FieldError[];
  /** Seconds, from `Retry-After`. Set on 429 and on the 409 that means the
   *  photo's variants are still being generated — both are answered by
   *  waiting, and neither should be retried immediately. */
  readonly retryAfterSeconds: number | null;

  constructor(init: {
    status: number;
    code: string;
    message: string;
    requestId: string | null;
    fieldErrors?: readonly FieldError[];
    retryAfterSeconds?: number | null;
  }) {
    super(init.message);
    this.name = new.target.name;
    this.status = init.status;
    this.code = init.code;
    this.requestId = init.requestId;
    this.fieldErrors = init.fieldErrors ?? [];
    this.retryAfterSeconds = init.retryAfterSeconds ?? null;
  }

  /** The message attached to one input, for a form that wants to show it
   *  beside the field rather than in a banner. */
  fieldError(field: string): string | undefined {
    return this.fieldErrors.find((error) => error.field === field)?.message;
  }
}

/**
 * A 401.
 *
 * Its own class because "you are not signed in" is not a failure of the thing
 * the user asked for — it is a different screen, and often no screen at all:
 * most of this application is readable signed out, so a 401 from a background
 * refresh means "show the signed-out state", not "show an error".
 */
export class UnauthorizedError extends ApiRequestError {}

/** Something below HTTP: the request never produced a response. Offline, DNS,
 *  a connection reset, or an aborted fetch. */
export class NetworkError extends Error {
  readonly cause: unknown;

  constructor(message: string, cause: unknown) {
    super(message);
    this.name = "NetworkError";
    this.cause = cause;
  }
}

export function isApiError(error: unknown): error is ApiRequestError {
  return error instanceof ApiRequestError;
}

/** Whether a caller should be asked to sign in rather than shown a failure. */
export function isUnauthorized(error: unknown): error is UnauthorizedError {
  return error instanceof UnauthorizedError;
}

// ---------------------------------------------------------------------------
// The wrapper
// ---------------------------------------------------------------------------

/** Prefix for every path. Empty because the dev server proxies `/api` and
 *  `/auth` to the service, which is what keeps the session cookie on one
 *  origin — see `vite.config.ts`. */
const BASE = "";

const REQUEST_ID_HEADER = "x-request-id";

export interface RequestOptions {
  signal?: AbortSignal;
}

interface Call extends RequestOptions {
  method: string;
  path: string;
  /** Serialized as JSON. `undefined` sends no body at all, which matters for
   *  GET and DELETE: a body on those confuses proxies for no gain. */
  body?: unknown;
  query?: QueryParams;
}

type QueryParams = Record<string, string | number | boolean | undefined>;

function withQuery(path: string, query: QueryParams | undefined): string {
  if (!query) return path;

  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    // An absent parameter and an empty one are different requests: `limit=`
    // is a parse failure server-side, while omitting it takes the default.
    if (value !== undefined) search.set(key, String(value));
  }

  const rendered = search.toString();
  return rendered ? `${path}?${rendered}` : path;
}

function requestIdOf(headers: Headers): string | null {
  return headers.get(REQUEST_ID_HEADER);
}

function retryAfterOf(value: string | null): number | null {
  if (value === null) return null;
  const seconds = Number(value);
  // `Retry-After` may also be an HTTP-date. This service only ever sends
  // seconds, and guessing at a date we do not send would be a parser with no
  // caller.
  return Number.isFinite(seconds) ? seconds : null;
}

/** Reads an error body, tolerating one that is not JSON at all. */
async function errorBodyOf(response: Response): Promise<ApiErrorBody> {
  const fallback: ApiErrorBody = {
    code: "unknown",
    message: response.statusText || "The request failed.",
  };

  let text: string;
  try {
    text = await response.text();
  } catch {
    return fallback;
  }
  if (!text) return fallback;

  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed === null || typeof parsed !== "object") return fallback;

    const body = parsed as Partial<ApiErrorBody>;
    return {
      code: typeof body.code === "string" ? body.code : fallback.code,
      message: typeof body.message === "string" ? body.message : fallback.message,
      errors: Array.isArray(body.errors) ? body.errors : undefined,
    };
  } catch {
    // An HTML error page from a proxy in front of the service, most likely.
    // The status is still the useful part.
    return fallback;
  }
}

/**
 * Turns a non-2xx response into the error a caller sees.
 *
 * Exported because the upload path builds its responses with `XMLHttpRequest`
 * rather than `fetch` — see {@link uploadPhotos} — and the two must not grow
 * different error shapes.
 */
export function errorFrom(
  status: number,
  body: ApiErrorBody,
  requestId: string | null,
  retryAfterSeconds: number | null = null,
): ApiRequestError {
  const init = {
    status,
    code: body.code,
    message: body.message,
    requestId,
    fieldErrors: body.errors,
    retryAfterSeconds,
  };

  return status === 401 ? new UnauthorizedError(init) : new ApiRequestError(init);
}

/**
 * One request, one response, or a throw.
 *
 * `T = void` for the endpoints that answer 204. Those are read as "no body"
 * rather than parsed, because `response.json()` on an empty body throws and
 * the throw would be reported as a failed delete that in fact succeeded.
 */
async function request<T>(call: Call): Promise<T> {
  const headers = new Headers({ accept: "application/json" });
  const hasBody = call.body !== undefined;
  if (hasBody) headers.set("content-type", "application/json");

  let response: Response;
  try {
    response = await fetch(BASE + withQuery(call.path, call.query), {
      method: call.method,
      headers,
      body: hasBody ? JSON.stringify(call.body) : undefined,
      // The whole reason this module exists. The session is an HttpOnly
      // cookie, and a fetch without this sends none — every authenticated
      // request would 401 and the app would look permanently signed out.
      credentials: "same-origin",
      signal: call.signal,
    });
  } catch (cause) {
    throw new NetworkError("The request could not be sent.", cause);
  }

  if (!response.ok) {
    throw errorFrom(
      response.status,
      await errorBodyOf(response),
      requestIdOf(response.headers),
      retryAfterOf(response.headers.get("retry-after")),
    );
  }

  if (response.status === 204 || response.status === 205) {
    return undefined as T;
  }

  const text = await response.text();
  if (!text) return undefined as T;

  return JSON.parse(text) as T;
}

// ---------------------------------------------------------------------------
// Voyages
// ---------------------------------------------------------------------------

export interface VoyageListParams extends RequestOptions {
  /** The caller's own voyages, drafts included, rather than the public feed.
   *  Requires a session; the feed does not. */
  mine?: boolean;
  /** The opaque `next_before` from the previous page. Do not construct one. */
  before?: string;
  limit?: number;
}

export function listVoyages(params: VoyageListParams = {}): Promise<VoyageList> {
  const { mine, before, limit, signal } = params;
  return request<VoyageList>({
    method: "GET",
    path: "/api/voyages",
    // `mine` is omitted rather than sent as `false`: the server defaults it,
    // and an explicit `mine=false` would look like a different cache key for
    // the same request.
    query: { mine: mine ? true : undefined, before, limit },
    signal,
  });
}

/** One voyage. Public when published; 404 — never 403 — when it is a draft
 *  belonging to somebody else. */
export function getVoyage(id: string, options: RequestOptions = {}): Promise<Voyage> {
  return request<Voyage>({ method: "GET", path: `/api/voyages/${id}`, ...options });
}

/** The lean payload the map draws from. Same visibility rule as
 *  {@link getVoyage}, and deliberately smaller: it is fetched per view. */
export function getVoyageMap(id: string, options: RequestOptions = {}): Promise<VoyageMap> {
  return request<VoyageMap>({ method: "GET", path: `/api/voyages/${id}/map`, ...options });
}

/** The rectangle the discovery map is looking at. */
export interface MapWindow {
  west: number;
  south: number;
  east: number;
  north: number;
  /** Web Mercator zoom. Decides which photographs the server groups together,
   *  so it is part of the question rather than a hint. */
  zoom: number;
}

/**
 * Every published voyage's photographs inside a rectangle, already grouped.
 *
 * No session, and the answer is the same for everyone. Unlike a single voyage's
 * map, this one cannot be sent whole and grouped in the browser — at a world
 * view the rectangle covers every published photograph there is.
 */
export function getDiscoverMap(
  window: MapWindow,
  options: RequestOptions = {},
): Promise<DiscoverMap> {
  const query = new URLSearchParams({
    west: String(window.west),
    south: String(window.south),
    east: String(window.east),
    north: String(window.north),
    zoom: String(window.zoom),
  });

  return request<DiscoverMap>({ method: "GET", path: `/api/map?${query}`, ...options });
}

export interface CreateVoyageInput {
  title?: string;
  description?: string;
  granularity?: Granularity;
}

/** Creates a draft. There is no `published` field on purpose — publishing is a
 *  separate, deliberate PATCH. */
export function createVoyage(
  input: CreateVoyageInput = {},
  options: RequestOptions = {},
): Promise<Voyage> {
  return request<Voyage>({ method: "POST", path: "/api/voyages", body: input, ...options });
}

export interface UpdateVoyageInput {
  title?: string;
  /** Three states, which is why this is not `string | undefined`: omit the key
   *  to leave the description alone, pass `null` to clear it, pass a string to
   *  set it. `JSON.stringify` drops `undefined` keys, so an omitted field
   *  really is absent on the wire. */
  description?: string | null;
  granularity?: Granularity;
  published?: boolean;
}

export function updateVoyage(
  id: string,
  patch: UpdateVoyageInput,
  options: RequestOptions = {},
): Promise<Voyage> {
  return request<Voyage>({
    method: "PATCH",
    path: `/api/voyages/${id}`,
    body: patch,
    ...options,
  });
}

/** Deletes a voyage and its photo records. */
export function deleteVoyage(id: string, options: RequestOptions = {}): Promise<void> {
  return request<void>({ method: "DELETE", path: `/api/voyages/${id}`, ...options });
}

/** Removes one photograph from a voyage. The photo record is deleted. */
export function detachPhoto(
  id: string,
  photoId: string,
  options: RequestOptions = {},
): Promise<void> {
  return request<void>({
    method: "DELETE",
    path: `/api/voyages/${id}/photos/${photoId}`,
    ...options,
  });
}

/** Sets the order. The body is the desired sequence in full, not a delta. */
export function reorderPhotos(
  id: string,
  photoIds: readonly string[],
  options: RequestOptions = {},
): Promise<Voyage> {
  return request<Voyage>({
    method: "PATCH",
    path: `/api/voyages/${id}/photos/order`,
    body: { photo_ids: photoIds },
    ...options,
  });
}

// ---------------------------------------------------------------------------
// Photos
// ---------------------------------------------------------------------------

export interface PhotoListParams extends RequestOptions {
  before?: string;
  limit?: number;
}

/** One voyage's owner-only photo metadata, newest first. Owner-only, and the coordinates in
 *  it are exact — never render them on a public page. */
export function listPhotos(voyageId: string, params: PhotoListParams = {}): Promise<PhotoList> {
  const { before, limit, signal } = params;
  return request<PhotoList>({
    method: "GET",
    path: `/api/voyages/${voyageId}/photos`,
    query: { before, limit },
    signal,
  });
}

export function getPhoto(id: string, options: RequestOptions = {}): Promise<Photo> {
  return request<Photo>({ method: "GET", path: `/api/photos/${id}`, ...options });
}

export function deletePhoto(id: string, options: RequestOptions = {}): Promise<void> {
  return request<void>({ method: "DELETE", path: `/api/photos/${id}`, ...options });
}

export interface UploadOptions extends RequestOptions {
  voyageId: string;
  /** Fraction between 0 and 1, or null while the total is unknown. Called as
   *  the body goes out, which is the only progress a browser can report. */
  onProgress?: (fraction: number | null) => void;
}

/**
 * Uploads one or more files.
 *
 * `XMLHttpRequest` rather than `fetch`, and not by preference: `fetch` reports
 * no upload progress in any shipping browser, and an upload screen with a bar
 * that only moves at the end is the design's "per-file progress" in name only.
 * The error path is routed through {@link errorFrom} so this call throws
 * exactly what every other call throws, request id included.
 *
 * The design's mixed-batch requirement (T8) is met by calling this **once per
 * file**: a batch is not a transaction server-side, but a rejected file fails
 * the request that carried it, so one request per file is what gives each file
 * its own outcome.
 */
export function uploadPhotos(
  files: readonly File[],
  options: UploadOptions,
): Promise<UploadResult> {
  const { onProgress, signal } = options;

  return new Promise<UploadResult>((resolve, reject) => {
    if (signal?.aborted) {
      reject(new NetworkError("The upload was canceled.", signal.reason));
      return;
    }

    const form = new FormData();
    for (const file of files) form.append("files", file, file.name);

    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${BASE}/api/voyages/${options.voyageId}/photos`);
    // Same-origin requests send cookies by default here, but a deployment that
    // ever moves the API to another origin would silently lose the session
    // without this. Stated rather than assumed, for the same reason
    // `credentials` is stated above.
    xhr.withCredentials = true;
    xhr.setRequestHeader("accept", "application/json");
    // Nothing sets `content-type`: the browser must write the multipart
    // boundary into it, and setting it by hand produces a header with no
    // boundary, which the server rejects as malformed.

    const finish = () => {
      signal?.removeEventListener("abort", onAbort);
    };

    const onAbort = () => {
      xhr.abort();
    };
    signal?.addEventListener("abort", onAbort, { once: true });

    if (onProgress) {
      xhr.upload.addEventListener("progress", (event) => {
        onProgress(event.lengthComputable ? event.loaded / event.total : null);
      });
    }

    xhr.addEventListener("abort", () => {
      finish();
      reject(new NetworkError("The upload was canceled.", signal?.reason));
    });

    xhr.addEventListener("error", () => {
      finish();
      reject(new NetworkError("The upload could not be sent.", null));
    });

    xhr.addEventListener("load", () => {
      finish();

      const requestId = xhr.getResponseHeader(REQUEST_ID_HEADER);

      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText) as UploadResult);
        } catch (cause) {
          reject(new NetworkError("The upload response could not be read.", cause));
        }
        return;
      }

      reject(
        errorFrom(
          xhr.status,
          parseErrorText(xhr.responseText, xhr.statusText),
          requestId,
          retryAfterOf(xhr.getResponseHeader("retry-after")),
        ),
      );
    });

    xhr.send(form);
  });
}

/** The `XMLHttpRequest` half of {@link errorBodyOf}, which cannot be shared
 *  because that one is async over a `Response`. */
function parseErrorText(text: string, statusText: string): ApiErrorBody {
  const fallback: ApiErrorBody = {
    code: "unknown",
    message: statusText || "The upload failed.",
  };
  if (!text) return fallback;

  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed === null || typeof parsed !== "object") return fallback;

    const body = parsed as Partial<ApiErrorBody>;
    return {
      code: typeof body.code === "string" ? body.code : fallback.code,
      message: typeof body.message === "string" ? body.message : fallback.message,
      errors: Array.isArray(body.errors) ? body.errors : undefined,
    };
  } catch {
    return fallback;
  }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/** The current user and their linked providers. 401 when signed out, which is
 *  an ordinary answer here rather than a failure — see `useMe`. */
export function getMe(options: RequestOptions = {}): Promise<Me> {
  return request<Me>({ method: "GET", path: "/api/me", ...options });
}

/**
 * Signs out.
 *
 * The server deletes the session row, so this is not merely a cookie being
 * cleared: a token someone else is holding stops working too. Answers 204 even
 * when there was no session, which is why it is `void` and never throws a 401.
 */
export function signOut(options: RequestOptions = {}): Promise<void> {
  return request<void>({ method: "POST", path: "/auth/logout", ...options });
}

/** Where a provider sign-in begins. A full-page navigation, not a fetch: the
 *  flow is a redirect chain that ends at the provider's consent screen. */
export function signInUrl(
  provider: "google" | "microsoft" | "apple" | "facebook",
  redirectTo?: string,
): string {
  const query = redirectTo ? `?${new URLSearchParams({ redirect_to: redirectTo })}` : "";
  return `${BASE}/auth/${provider}/start${query}`;
}

// ---------------------------------------------------------------------------

/** Every method, for a call site that would rather hold one object than a
 *  dozen imports. The named exports above are the same functions. */
export const api = {
  listVoyages,
  getVoyage,
  getVoyageMap,
  getDiscoverMap,
  createVoyage,
  updateVoyage,
  deleteVoyage,
  detachPhoto,
  reorderPhotos,
  listPhotos,
  getPhoto,
  uploadPhotos,
  deletePhoto,
  getMe,
  signOut,
  signInUrl,
} as const;
