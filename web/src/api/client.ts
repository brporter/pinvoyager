/**
 * HTTP transports for JSON requests and multipart uploads.
 *
 * JSON requests use fetch; uploads use XMLHttpRequest for progress events.
 * Both send session cookies and preserve request IDs on API errors. HTTP 401
 * throws UnauthorizedError; transport failures throw NetworkError. Response
 * types are checked against captured API fixtures.
 */

import type {
  AdminAction,
  AdminUserPage,
  AdminUserDetail,
  DiscoverMap,
  Granularity,
  Me,
  PublicProfilePage,
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
 * Parsed error fields. Unknown server codes are retained as strings;
 * missing or malformed fields receive fallbacks in the transport helpers.
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

/** HTTP 401 response, distinguished so callers can request authentication. */
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

/** Owned by one creation/upload workflow. Each JSON response replaces the
 * parent so later uploads join that workflow trace. Never share it across
 * independent workflows or persist it beyond the screen lifetime. */
export interface TraceContext {
  traceparent?: string;
}

export interface RequestOptions {
  traceContext?: TraceContext;
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
  // Parse Retry-After as seconds, the format emitted by this API.
  return Number.isFinite(seconds) ? seconds : null;
}

/** Reads an error body, tolerating one that is not JSON at all. */
async function errorBodyOf(response: Response): Promise<ApiErrorBody> {
  const fallback = response.statusText || "The request failed.";
  try {
    return parseErrorText(await response.text(), fallback);
  } catch {
    return parseErrorText("", fallback);
  }
}

/**
 * Converts an HTTP error response into ApiRequestError or UnauthorizedError,
 * including its request ID, validation fields, and retry delay.
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
 * Sends a JSON request and returns its decoded response. HTTP 204 returns
 * undefined without parsing JSON. Non-success statuses and transport failures
 * throw typed errors.
 */
async function request<T>(call: Call): Promise<T> {
  const headers = new Headers({ accept: "application/json" });
  const hasBody = call.body !== undefined;
  if (hasBody) headers.set("content-type", "application/json");
  if (call.traceContext?.traceparent) headers.set("traceparent", call.traceContext.traceparent);

  let response: Response;
  try {
    response = await fetch(BASE + withQuery(call.path, call.query), {
      method: call.method,
      headers,
      body: hasBody ? JSON.stringify(call.body) : undefined,
      // Explicitly send cookies for same-origin API requests.
      credentials: "same-origin",
      signal: call.signal,
    });
  } catch (cause) {
    throw new NetworkError("The request could not be sent.", cause);
  }

  if (call.traceContext) {
    const parent = response.headers.get("traceparent");
    if (parent) call.traceContext.traceparent = parent;
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
    // Omit mine when false; the server defaults to the public feed.
    query: { mine: mine ? true : undefined, before, limit },
    signal,
  });
}

/** One voyage. Public when published; 404 — never 403 — when it is a draft
 *  belonging to somebody else. */
export function getVoyage(id: string, options: RequestOptions = {}): Promise<Voyage> {
  return request<Voyage>({ method: "GET", path: `/api/voyages/${id}`, ...options });
}

/** Fetches voyage map data using the same visibility rules as getVoyage. */
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
 * Fetches server-grouped published photo locations in the viewport.
 * The endpoint does not require a session.
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

/** Orders listed photos first; omitted photos retain their relative order afterward. */
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

/**
 * Fetches one voyage's owner-only photo metadata in descending ID order,
 * including exact coordinates.
 */
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
  /** Reuse for retries of the same files in the same order. */
  idempotencyKey?: string;
  voyageId: string;
  /**
   * Fraction of request bytes sent, from 0 to 1, or null when total size is
   * unknown. Does not report server processing or variant generation progress.
   */
  onProgress?: (fraction: number | null) => void;
}

/**
 * Uploads files using XMLHttpRequest and reports upload progress.
 *
 * HTTP and transport failures throw the same error classes as JSON requests.
 * The batch hook calls this once per file so each file has its own outcome.
 * A successful response accepts the photo; variants may still be pending.
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
    // Include cookies with the upload request.
    xhr.withCredentials = true;
    xhr.setRequestHeader("accept", "application/json");
    if (options.idempotencyKey) xhr.setRequestHeader("idempotency-key", options.idempotencyKey);
    if (options.traceContext?.traceparent)
      xhr.setRequestHeader("traceparent", options.traceContext.traceparent);
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
          parseErrorText(xhr.responseText, xhr.statusText || "The upload failed."),
          requestId,
          retryAfterOf(xhr.getResponseHeader("retry-after")),
        ),
      );
    });

    xhr.send(form);
  });
}

/** Shared error-body parsing for fetch and XHR; transport code supplies the fallback message. */
function parseErrorText(text: string, fallbackMessage: string): ApiErrorBody {
  const fallback: ApiErrorBody = {
    code: "unknown",
    message: fallbackMessage,
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

/** Claim or rename the unique public display name. */
export function updateDisplayName(displayName: string): Promise<void> {
  return request<void>({ method: "PATCH", path: "/api/me", body: { display_name: displayName } });
}

export function getPublicProfile(
  name: string,
  options: RequestOptions & { before?: string; limit?: number } = {},
): Promise<PublicProfilePage> {
  const { before, limit, signal } = options;
  return request<PublicProfilePage>({
    method: "GET",
    path: `/api/profiles/${encodeURIComponent(name)}`,
    query: { before, limit },
    signal,
  });
}

/**
 * Revokes the presented session and clears its cookie. Success returns no
 * body, including when no session exists. HTTP or transport failures throw.
 */
export function signOut(options: RequestOptions = {}): Promise<void> {
  return request<void>({ method: "POST", path: "/auth/logout", ...options });
}

/** Where a provider sign-in begins. A full-page navigation, not a fetch: the
 *  flow is a redirect chain that ends at the provider's consent screen. */
export function signInUrl(provider: "google" | "microsoft" | "apple", redirectTo?: string): string {
  const query = redirectTo ? `?${new URLSearchParams({ redirect_to: redirectTo })}` : "";
  return `${BASE}/auth/${provider}/start${query}`;
}

// ---------------------------------------------------------------------------

/** Every method, for a call site that would rather hold one object than a
 *  dozen imports. The named exports above are the same functions. */
export const api = {
  getAdminUsers,
  getAdminUser,
  administerUser,
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
  updateDisplayName,
  getPublicProfile,
  signOut,
  signInUrl,
} as const;

/** Administrator metadata; the server checks the current account role on each call. */
export function getAdminUsers(before?: string): Promise<AdminUserPage> {
  return request({ method: "GET", path: "/api/admin/users", query: { before } });
}
export function getAdminUser(id: string, before?: string): Promise<AdminUserDetail> {
  return request({
    method: "GET",
    path: `/api/admin/users/${encodeURIComponent(id)}`,
    query: { before },
  });
}
export function administerUser(id: string, body: AdminAction): Promise<void> {
  return request({
    method: "POST",
    path: `/api/admin/users/${encodeURIComponent(id)}/actions`,
    body,
  });
}
