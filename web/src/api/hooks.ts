/**
 * React Query bindings for the API client.
 *
 * The client in `./client` knows how to make a request; this file knows what a
 * request means to the cache. Two decisions run through all of it:
 *
 *  * **Keys are hierarchical**, so invalidation can be coarse where coarse is
 *    correct. `["voyages"]` is a prefix of every voyage key, which means
 *    publishing a voyage can invalidate the feed, the detail and the map with
 *    one call instead of three that have to be kept in step with the routes.
 *  * **A 4xx is not retried.** React Query's default is three attempts, which
 *    for a 404 or a 401 means three identical failures and a screen that takes
 *    a second and a half to say "not found". Only 5xx and network faults are
 *    worth a second try.
 */

import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
  type UseInfiniteQueryResult,
  type UseMutationResult,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  createVoyage,
  deletePhoto,
  deleteVoyage,
  detachPhoto,
  getMe,
  getPhoto,
  getVoyage,
  getVoyageMap,
  isApiError,
  isUnauthorized,
  listPhotos,
  listVoyages,
  reorderPhotos,
  signOut,
  updateVoyage,
  uploadPhotos,
  getDiscoverMap,
  type CreateVoyageInput,
  type MapWindow,
  type UpdateVoyageInput,
  type UploadOptions,
} from "./client";
import type {
  DiscoverMap,
  Me,
  Photo,
  PhotoList,
  UploadResult,
  Voyage,
  VoyageList,
  VoyageMap,
} from "./types";

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/**
 * Every cache key in the application, in one place.
 *
 * Written as prefix-sharing tuples rather than strings so that
 * `invalidateQueries({ queryKey: keys.voyages.all })` reaches the feed, every
 * detail read and every map. A flat `"voyage-detail-<id>"` scheme cannot
 * express that, and the invalidation lists then drift from the routes.
 */
export const keys = {
  me: ["me"] as const,

  voyages: {
    all: ["voyages"] as const,
    lists: () => [...keys.voyages.all, "list"] as const,
    list: (params: VoyageFeedParams) => [...keys.voyages.lists(), params] as const,
    detail: (id: string) => [...keys.voyages.all, "detail", id] as const,
    map: (id: string) => [...keys.voyages.all, "map", id] as const,
  },

  /** The discovery map, keyed by the rectangle and zoom it was asked for.
   *
   *  Deliberately outside `voyages`, even though it draws voyages. Publishing
   *  one voyage invalidates `["voyages"]`, and every rectangle of the world map
   *  a user had panned across would be thrown away with it — while the answer
   *  they are looking at changed by at most one marker. */
  discoverMap: (window: MapWindow) => ["discover-map", window] as const,

  photos: {
    all: ["photos"] as const,
    lists: () => [...keys.photos.all, "list"] as const,
    list: (params: PhotoFeedParams) => [...keys.photos.lists(), params] as const,
    detail: (id: string) => [...keys.photos.all, "detail", id] as const,
  },
} as const;

// ---------------------------------------------------------------------------
// Shared policy
// ---------------------------------------------------------------------------

/**
 * Retry only what a retry could fix.
 *
 * A 404 does not become a 200 on the third attempt, and a 401 certainly does
 * not. Retrying them costs the user the latency of every attempt before the
 * designed empty or signed-out state appears. 5xx and network faults are
 * genuinely transient, so those keep the usual budget.
 */
export function retryOnlyTransient(failureCount: number, error: Error): boolean {
  if (isApiError(error) && error.status < 500) return false;
  return failureCount < 2;
}

const queryDefaults = { retry: retryOnlyTransient } as const;

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/**
 * The current user, or `null` when signed out.
 *
 * The 401 is swallowed on purpose. Most of this application is readable
 * without an account, so "not signed in" is an ordinary state of the shell and
 * not an error for it to render — a `useMe` that surfaced a 401 as `isError`
 * would put an error boundary in front of the discovery feed for every visitor
 * who has not signed in.
 */
export function useMe(): UseQueryResult<Me | null> {
  return useQuery({
    queryKey: keys.me,
    queryFn: async ({ signal }) => {
      try {
        return await getMe({ signal });
      } catch (error) {
        if (isUnauthorized(error)) return null;
        throw error;
      }
    },
    ...queryDefaults,
  });
}

/**
 * Signs out, then empties the cache.
 *
 * `clear` rather than a list of invalidations: anything fetched while signed in
 * may have been visible only to that session — the caller's own drafts, their
 * owner-only photos and exact coordinates — and a cache that kept them would leave
 * the previous user's data on screen for the next one.
 */
export function useSignOut(): UseMutationResult<void, Error, void> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: () => signOut(),
    onSuccess: () => {
      client.setQueryData(keys.me, null);
      client.clear();
    },
  });
}

// ---------------------------------------------------------------------------
// Voyages
// ---------------------------------------------------------------------------

export interface VoyageFeedParams {
  /** The caller's own voyages, drafts included. Part of the key because the
   *  feed and the owner's list are different results from one path. */
  mine?: boolean;
  limit?: number;
}

/**
 * The discovery feed, or the caller's own list, paged.
 *
 * Infinite rather than page-numbered because the API is cursor-paginated: it
 * hands back an opaque `next_before`, and there is no page number to ask for.
 * That is deliberate server-side — a voyage published between two requests
 * shifts every offset, so an offset boundary skips or repeats a voyage.
 */
export function useVoyages(
  params: VoyageFeedParams = {},
): UseInfiniteQueryResult<InfiniteData<VoyageList, string | null>> {
  return useInfiniteQuery({
    queryKey: keys.voyages.list(params),
    queryFn: ({ pageParam, signal }) =>
      listVoyages({
        mine: params.mine,
        limit: params.limit,
        before: pageParam ?? undefined,
        signal,
      }),
    initialPageParam: null as string | null,
    // `null` ends the list. Returning `undefined` is what React Query reads as
    // "no more pages", so the null has to be mapped rather than passed on.
    getNextPageParam: (last: VoyageList) => last.next_before ?? undefined,
    ...queryDefaults,
  });
}

export function useVoyage(id: string | undefined): UseQueryResult<Voyage> {
  return useQuery({
    queryKey: keys.voyages.detail(id ?? ""),
    queryFn: ({ signal }) => getVoyage(id as string, { signal }),
    enabled: id !== undefined,
    ...queryDefaults,
    // Variant generation finishes after ingestion. Refresh until all photos settle.
    refetchInterval: (query) =>
      query.state.data?.photos?.some((photo) => photo.variant_status === "pending")
        ? 1000
        : false,
  });
}

export function useVoyageMap(id: string | undefined): UseQueryResult<VoyageMap> {
  return useQuery({
    queryKey: keys.voyages.map(id ?? ""),
    queryFn: ({ signal }) => getVoyageMap(id as string, { signal }),
    enabled: id !== undefined,
    ...queryDefaults,
  });
}

/**
 * The discovery map for one rectangle.
 *
 * `enabled` because the rectangle is not known until the map has laid itself
 * out and reported where it is looking — asking before then would request an
 * irrelevant window from our own discovery API. These marker requests are
 * separate from MapLibre's requests to OSM's tile server.
 *
 * `placeholderData` keeps the previous rectangle's markers on screen while the
 * next one loads. Without it every pan blanks the map and redraws it, which
 * reads as the pins flickering rather than as data arriving.
 */
export function useDiscoverMap(
  window: MapWindow | null,
): UseQueryResult<DiscoverMap> {
  return useQuery({
    queryKey: keys.discoverMap(window ?? EMPTY_WINDOW),
    queryFn: ({ signal }) => getDiscoverMap(window as MapWindow, { signal }),
    enabled: window !== null,
    placeholderData: (previous) => previous,
    ...queryDefaults,
  });
}

/** Never fetched — `enabled` is false whenever it would be the key. It exists
 *  so the key is a stable shape rather than sometimes containing null. */
const EMPTY_WINDOW: MapWindow = { west: 0, south: 0, east: 0, north: 0, zoom: 0 };

/**
 * Puts a freshly read voyage into the cache and invalidates what it changed.
 *
 * Every mutating voyage endpoint answers with the whole voyage, so seeding the
 * detail key means the edit screen re-renders from the response rather than
 * from a second round trip. The lists and the map are invalidated rather than
 * written: a patch can change the title (the feed), the granularity (every
 * pin) or the published flag (whether it is in the feed at all), and guessing
 * which is how a stale list survives an edit.
 */
function voyageChanged(client: QueryClient, voyage: Voyage): void {
  client.setQueryData(keys.voyages.detail(voyage.id), voyage);
  void client.invalidateQueries({ queryKey: keys.voyages.lists() });
  void client.invalidateQueries({ queryKey: keys.voyages.map(voyage.id) });
}

export function useCreateVoyage(): UseMutationResult<Voyage, Error, CreateVoyageInput> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateVoyageInput) => createVoyage(input),
    onSuccess: (voyage) => voyageChanged(client, voyage),
  });
}

export function useUpdateVoyage(
  id: string,
): UseMutationResult<Voyage, Error, UpdateVoyageInput> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (patch: UpdateVoyageInput) => updateVoyage(id, patch),
    onSuccess: (voyage) => voyageChanged(client, voyage),
  });
}

export function useDeleteVoyage(): UseMutationResult<void, Error, string> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteVoyage(id),
    onSuccess: (_result, id) => {
      client.removeQueries({ queryKey: keys.voyages.detail(id) });
      client.removeQueries({ queryKey: keys.voyages.map(id) });
      void client.invalidateQueries({ queryKey: keys.voyages.lists() });
      // The voyage and its photo records were deleted.
      void client.invalidateQueries({ queryKey: keys.photos.all });
    },
  });
}

export function useDetachPhoto(voyageId: string): UseMutationResult<void, Error, string> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (photoId: string) => detachPhoto(voyageId, photoId),
    onSuccess: () => {
      // 204, so there is no voyage to seed with — the detail read has to be
      // refetched rather than patched.
      void client.invalidateQueries({ queryKey: keys.voyages.detail(voyageId) });
      void client.invalidateQueries({ queryKey: keys.voyages.map(voyageId) });
      void client.invalidateQueries({ queryKey: keys.photos.all });
    },
  });
}

export function useReorderPhotos(
  voyageId: string,
): UseMutationResult<Voyage, Error, readonly string[]> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (photoIds: readonly string[]) => reorderPhotos(voyageId, photoIds),
    onSuccess: (voyage) => voyageChanged(client, voyage),
  });
}

// ---------------------------------------------------------------------------
// Photos
// ---------------------------------------------------------------------------

export interface PhotoFeedParams {
  voyageId: string;
  limit?: number;
}

/** One voyage's owner-only photos, paged. Cursor-paginated for the same reason the
 *  voyage feed is: an upload landing mid-scroll would shift every offset. */
export function usePhotos(
  params: PhotoFeedParams,
): UseInfiniteQueryResult<InfiniteData<PhotoList, string | null>> {
  return useInfiniteQuery({
    queryKey: keys.photos.list(params),
    queryFn: ({ pageParam, signal }) =>
      listPhotos(params.voyageId, { limit: params.limit, before: pageParam ?? undefined, signal }),
    initialPageParam: null as string | null,
    getNextPageParam: (last: PhotoList) => last.next_before ?? undefined,
    ...queryDefaults,
  });
}

export function usePhoto(id: string | undefined): UseQueryResult<Photo> {
  return useQuery({
    queryKey: keys.photos.detail(id ?? ""),
    queryFn: ({ signal }) => getPhoto(id as string, { signal }),
    enabled: id !== undefined,
    ...queryDefaults,
  });
}

export interface UploadVariables extends UploadOptions {
  files: readonly File[];
}

/**
 * Uploads a batch.
 *
 * Not retried, ever. A retry would re-send the bytes, and the failure that
 * makes an upload worth retrying — a 413, a 415, a 429 — is one where sending
 * them again is either pointless or the thing being rate-limited.
 *
 * The uploaded photos belong to their voyage with `variant_status: "pending"`, so
 * a caller that immediately renders their thumbnails will get a 409 until the
 * worker catches up. That is a state to draw, not a race to avoid.
 */
export function useUploadPhotos(): UseMutationResult<UploadResult, Error, UploadVariables> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: ({ files, ...options }: UploadVariables) => uploadPhotos(files, options),
    retry: false,
    onSuccess: (_result, { voyageId }) => {
      void client.invalidateQueries({ queryKey: keys.voyages.detail(voyageId) });
      void client.invalidateQueries({ queryKey: keys.voyages.map(voyageId) });
      void client.invalidateQueries({ queryKey: keys.voyages.lists() });
      void client.invalidateQueries({ queryKey: keys.photos.lists() });
    },
  });
}

export function useDeletePhoto(): UseMutationResult<void, Error, string> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deletePhoto(id),
    onSuccess: (_result, id) => {
      client.removeQueries({ queryKey: keys.photos.detail(id) });
      void client.invalidateQueries({ queryKey: keys.photos.lists() });
      // A deleted photograph disappears from every voyage it was in, so no
      // voyage read that mentioned it is still correct.
      void client.invalidateQueries({ queryKey: keys.voyages.all });
    },
  });
}
