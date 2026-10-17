/**
 * React Query keys, queries, and mutations for the API client.
 * Keys share prefixes for related voyage reads. Mutations update detail data
 * and invalidate affected lists, profiles, and discovery queries. Retries are
 * limited to server errors and network failures.
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
  getPublicProfile,
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
  PublicProfilePage,
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

/** Shared query keys. Prefixes let callers invalidate related voyage queries. */
export const keys = {
  me: ["me"] as const,

  voyages: {
    all: ["voyages"] as const,
    lists: () => [...keys.voyages.all, "list"] as const,
    list: (params: VoyageFeedParams) => [...keys.voyages.lists(), params] as const,
    detail: (id: string) => [...keys.voyages.all, "detail", id] as const,
    map: (id: string) => [...keys.voyages.all, "map", id] as const,
  },

  /** Discovery queries use their own prefix and include the viewport and zoom.
   *  `invalidatePublicViews` invalidates them after voyage/photo mutations. */
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

/** Retries errors up to twice, except API responses with a status below 500. */
export function retryOnlyTransient(failureCount: number, error: Error): boolean {
  if (isApiError(error) && error.status < 500) return false;
  return failureCount < 2;
}

const queryDefaults = { retry: retryOnlyTransient } as const;

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/** Reads the current user. HTTP 401 becomes null; other failures remain query errors. */
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
 * Signs out and clears all query data on success, including cached drafts
 * and exact owner metadata. Failures leave the cache unchanged.
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

/** Reads public or owner voyages using the API's next_before cursor. */
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
    refetchInterval: (query) =>
      query.state.data?.pages.some((page) => page.voyages.some((voyage) => voyage.photos_pending))
        ? 1000
        : false,
  });
}

/** Every page is filtered to this creator by ID or name on the server. */
export function usePublicProfile(
  name: string | undefined,
): UseInfiniteQueryResult<InfiniteData<PublicProfilePage, string | null>> {
  return useInfiniteQuery({
    queryKey: ["public-profile", name ?? ""],
    queryFn: ({ pageParam, signal }) =>
      getPublicProfile(name as string, { before: pageParam ?? undefined, signal }),
    enabled: name !== undefined,
    initialPageParam: null as string | null,
    getNextPageParam: (last: PublicProfilePage) => last.next_before ?? undefined,
    ...queryDefaults,
    refetchInterval: (query) =>
      query.state.data?.pages.some((page) => page.voyages.some((voyage) => voyage.photos_pending))
        ? 1000
        : false,
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
      query.state.data?.photos?.some((photo) => photo.variant_status === "pending") ? 1000 : false,
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
 * Fetches discovery clusters once a viewport is available. Keeps the previous
 * viewport's results visible while the next request loads.
 */
export function useDiscoverMap(window: MapWindow | null): UseQueryResult<DiscoverMap> {
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

/** Public maps and profile pages can change after any voyage/photo mutation. */
function invalidatePublicViews(client: QueryClient): void {
  void client.invalidateQueries({ queryKey: ["discover-map"] });
  void client.invalidateQueries({ queryKey: ["public-profile"] });
  void client.invalidateQueries({ queryKey: keys.voyages.lists() });
}

/** Creation, edits, and reordering return a complete voyage. Store that detail
 *  response and invalidate related lists and maps so they fetch current data. */
function voyageChanged(client: QueryClient, voyage: Voyage): void {
  invalidatePublicViews(client);
  client.setQueryData(keys.voyages.detail(voyage.id), voyage);
  void client.invalidateQueries({ queryKey: keys.voyages.lists() });
  void client.invalidateQueries({ queryKey: keys.voyages.map(voyage.id) });
}

export function useCreateVoyage(
  options?: import("./client").RequestOptions,
): UseMutationResult<Voyage, Error, CreateVoyageInput> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateVoyageInput) =>
      options === undefined ? createVoyage(input) : createVoyage(input, options),
    onSuccess: (voyage) => voyageChanged(client, voyage),
  });
}

export function useUpdateVoyage(id: string): UseMutationResult<Voyage, Error, UpdateVoyageInput> {
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
      invalidatePublicViews(client);
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
      invalidatePublicViews(client);
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
 * Uploads without automatic retries; callers control per-file retry actions.
 * Accepted photos can remain pending until the worker generates their variants.
 */
export function useUploadPhotos(): UseMutationResult<UploadResult, Error, UploadVariables> {
  const client = useQueryClient();

  return useMutation({
    mutationFn: ({ files, ...options }: UploadVariables) => uploadPhotos(files, options),
    retry: false,
    onSuccess: (_result, { voyageId }) => {
      invalidatePublicViews(client);
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
      invalidatePublicViews(client);
      client.removeQueries({ queryKey: keys.photos.detail(id) });
      void client.invalidateQueries({ queryKey: keys.photos.lists() });
      // Invalidate voyage data that may include the deleted photo.
      void client.invalidateQueries({ queryKey: keys.voyages.all });
    },
  });
}
