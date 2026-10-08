import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  createVoyage: vi.fn(),
  deletePhoto: vi.fn(),
  deleteVoyage: vi.fn(),
  detachPhoto: vi.fn(),
  getDiscoverMap: vi.fn(),
  getMe: vi.fn(),
  getPhoto: vi.fn(),
  getVoyage: vi.fn(),
  getVoyageMap: vi.fn(),
  isApiError: vi.fn(),
  isUnauthorized: vi.fn(),
  listPhotos: vi.fn(),
  listVoyages: vi.fn(),
  reorderPhotos: vi.fn(),
  signOut: vi.fn(),
  updateVoyage: vi.fn(),
  uploadPhotos: vi.fn(),
}));

vi.mock("../client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../client")>();
  return { ...actual, ...mocks };
});

import {
  keys,
  retryOnlyTransient,
  useCreateVoyage,
  useDeletePhoto,
  useDeleteVoyage,
  useDetachPhoto,
  useDiscoverMap,
  useMe,
  usePhoto,
  usePhotos,
  useReorderPhotos,
  useSignOut,
  useUpdateVoyage,
  useUploadPhotos,
  useVoyage,
  useVoyageMap,
  useVoyages,
} from "../hooks";
import { ApiRequestError, UnauthorizedError } from "../client";
import type { Voyage } from "../types";

function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, wrapper };
}

const voyage = { id: "v1", title: "Trip" } as Voyage;

afterEach(() => {
  vi.clearAllMocks();
});

describe("voyage image processing", () => {
  beforeEach(() => { vi.useFakeTimers(); mocks.getVoyage.mockReset(); });
  afterEach(() => { vi.useRealTimers(); });

  const state = (id: string, statuses: string[]) => ({
    ...voyage, id, photos: statuses.map((variant_status, index) => ({ id: `p${index}`, variant_status })),
  }) as Voyage;
  async function tick(ms = 20) {
    await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
  }

  it("continues through staggered completion and stops after ready or failed results", async () => {
    const { client, wrapper } = setup();
    mocks.getVoyage.mockResolvedValueOnce(state("v1", ["ready", "pending", "pending"]))
      .mockResolvedValueOnce(state("v1", ["ready", "ready", "pending"]))
      .mockResolvedValue(state("v1", ["ready", "ready", "failed"]));
    const hook = renderHook(() => useVoyage("v1"), { wrapper });
    await tick();
    expect(hook.result.current.data?.photos[1].variant_status).toBe("pending");
    await tick(1000);
    expect(hook.result.current.data?.photos[1].variant_status).toBe("ready");
    expect(hook.result.current.data?.photos[2].variant_status).toBe("pending");
    await tick(1000);
    expect(hook.result.current.data?.photos[2].variant_status).toBe("failed");
    await tick(5000);
    expect(mocks.getVoyage).toHaveBeenCalledTimes(3);
    hook.unmount(); client.clear();
  });

  it("stops polling on unmount", async () => {
    const { client, wrapper } = setup();
    mocks.getVoyage.mockResolvedValue(state("v1", ["pending"]));
    const hook = renderHook(() => useVoyage("v1"), { wrapper });
    await tick();
    hook.unmount();
    await tick(5000);
    expect(mocks.getVoyage).toHaveBeenCalledTimes(1);
    client.clear();
  });

  it("recovers after a temporary polling failure", async () => {
    const { client, wrapper } = setup();
    mocks.getVoyage.mockResolvedValueOnce(state("v1", ["pending"]))
      .mockRejectedValueOnce(new Error("temporary network failure"))
      .mockResolvedValue(state("v1", ["ready"]));
    const hook = renderHook(() => useVoyage("v1"), { wrapper });
    await tick();
    await tick(1000);
    await tick(10_000);
    expect(hook.result.current.data?.photos[0].variant_status).toBe("ready");
    const calls = mocks.getVoyage.mock.calls.length;
    await tick(5000);
    expect(mocks.getVoyage).toHaveBeenCalledTimes(calls);
    hook.unmount(); client.clear();
  });

  it("does not apply a late response to another voyage", async () => {
    const { client, wrapper } = setup();
    let finish!: (value: Voyage) => void;
    mocks.getVoyage.mockImplementation((id: string) => id === "v1"
      ? new Promise<Voyage>(resolve => { finish = resolve; })
      : Promise.resolve(state("v2", ["ready"])));
    const hook = renderHook(({ id }) => useVoyage(id), { wrapper, initialProps: { id: "v1" } });
    await tick();
    hook.rerender({ id: "v2" });
    await tick();
    expect(hook.result.current.data?.id).toBe("v2");
    await act(async () => { finish(state("v1", ["pending"])); });
    await tick(5000);
    expect(hook.result.current.data?.id).toBe("v2");
    expect(mocks.getVoyage).toHaveBeenCalledTimes(2);
    hook.unmount(); client.clear();
  });
});

describe("query keys and retry policy", () => {
  it("groups voyage and photo keys by invalidation scope", () => {
    expect(keys.me).toEqual(["me"]);
    expect(keys.voyages.all).toEqual(["voyages"]);
    expect(keys.voyages.lists()).toEqual(["voyages", "list"]);
    expect(keys.voyages.list({ mine: true, limit: 4 })).toEqual([
      "voyages",
      "list",
      { mine: true, limit: 4 },
    ]);
    expect(keys.voyages.detail("v1")).toEqual(["voyages", "detail", "v1"]);
    expect(keys.voyages.map("v1")).toEqual(["voyages", "map", "v1"]);
    expect(keys.photos.all).toEqual(["photos"]);
    expect(keys.photos.lists()).toEqual(["photos", "list"]);
    expect(keys.photos.list({ voyageId: "v1", limit: 8 })).toEqual(["photos", "list", { voyageId: "v1", limit: 8 }]);
    expect(keys.photos.detail("p1")).toEqual(["photos", "detail", "p1"]);
    expect(keys.discoverMap({ west: 1, south: 2, east: 3, north: 4, zoom: 5 })).toEqual([
      "discover-map",
      { west: 1, south: 2, east: 3, north: 4, zoom: 5 },
    ]);
  });

  it("retries transient failures twice but never retries client errors", () => {
    mocks.isApiError.mockImplementation((error) => error instanceof ApiRequestError);
    const notFound = new ApiRequestError({
      status: 404,
      code: "not_found",
      message: "Missing",
      requestId: null,
    });
    const serverError = new ApiRequestError({
      status: 503,
      code: "unavailable",
      message: "Unavailable",
      requestId: null,
    });

    expect(retryOnlyTransient(0, notFound)).toBe(false);
    expect(retryOnlyTransient(0, new UnauthorizedError({
      status: 401,
      code: "unauthorized",
      message: "Sign in",
      requestId: null,
    }))).toBe(false);
    expect(retryOnlyTransient(0, serverError)).toBe(true);
    expect(retryOnlyTransient(1, new Error("offline"))).toBe(true);
    expect(retryOnlyTransient(2, new Error("offline"))).toBe(false);
  });
});

describe("read hooks", () => {
  it("returns null for an unauthorized current-user read and preserves other errors", async () => {
    const { client, wrapper } = setup();
    mocks.isUnauthorized.mockImplementation((error) => error instanceof UnauthorizedError);
    mocks.getMe.mockRejectedValueOnce(new UnauthorizedError({
      status: 401,
      code: "unauthorized",
      message: "Sign in",
      requestId: null,
    }));
    const signedOut = renderHook(() => useMe(), { wrapper });
    await waitFor(() => expect(signedOut.result.current.isSuccess).toBe(true));
    expect(signedOut.result.current.data).toBeNull();
    signedOut.unmount();
    client.clear();

    mocks.isUnauthorized.mockReturnValue(false);
    mocks.getMe.mockRejectedValue(new Error("offline"));
    const broken = renderHook(() => useMe(), { wrapper: setup().wrapper });
    await waitFor(() => expect(broken.result.current.isError).toBe(true), { timeout: 5000 });
    expect(broken.result.current.error?.message).toBe("offline");
  }, 15_000);

  it("keeps voyage, map, photo, and discovery queries disabled until identifiers exist", async () => {
    const { wrapper } = setup();
    const noVoyage = renderHook(() => useVoyage(undefined), { wrapper });
    const noMap = renderHook(() => useVoyageMap(undefined), { wrapper });
    const noPhoto = renderHook(() => usePhoto(undefined), { wrapper });
    const noWindow = renderHook(() => useDiscoverMap(null), { wrapper });
    expect(noVoyage.result.current.fetchStatus).toBe("idle");
    expect(noMap.result.current.fetchStatus).toBe("idle");
    expect(noPhoto.result.current.fetchStatus).toBe("idle");
    expect(noWindow.result.current.fetchStatus).toBe("idle");

    mocks.getVoyage.mockResolvedValue(voyage);
    const enabled = renderHook(() => useVoyage("v1"), { wrapper });
    await waitFor(() => expect(enabled.result.current.data).toBe(voyage));
    expect(mocks.getVoyage).toHaveBeenCalledWith("v1", expect.objectContaining({ signal: expect.any(AbortSignal) }));
  });

  it("runs enabled map and photo queries and paginates the photo feed", async () => {
    const { wrapper } = setup();
    mocks.getVoyageMap.mockResolvedValue({} as never);
    mocks.getDiscoverMap.mockResolvedValue({} as never);
    mocks.getPhoto.mockResolvedValue({} as never);
    mocks.listPhotos
      .mockResolvedValueOnce({ photos: [], next_before: "photo-cursor" })
      .mockResolvedValueOnce({ photos: [], next_before: null });

    const voyageMap = renderHook(() => useVoyageMap("v1"), { wrapper });
    const discoverMap = renderHook(
      () => useDiscoverMap({ west: 1, south: 2, east: 3, north: 4, zoom: 5 }),
      { wrapper },
    );
    const photo = renderHook(() => usePhoto("p1"), { wrapper });
    const photos = renderHook(() => usePhotos({ voyageId: "v1", limit: 7 }), { wrapper });

    await waitFor(() => {
      expect(voyageMap.result.current.isSuccess).toBe(true);
      expect(discoverMap.result.current.isSuccess).toBe(true);
      expect(photo.result.current.isSuccess).toBe(true);
      expect(photos.result.current.isSuccess).toBe(true);
    });

    expect(mocks.getVoyageMap).toHaveBeenCalledWith("v1", expect.any(Object));
    expect(mocks.getDiscoverMap).toHaveBeenCalledWith(
      { west: 1, south: 2, east: 3, north: 4, zoom: 5 },
      expect.any(Object),
    );
    expect(mocks.getPhoto).toHaveBeenCalledWith("p1", expect.any(Object));
    expect(photos.result.current.hasNextPage).toBe(true);

    await act(async () => {
      await photos.result.current.fetchNextPage();
    });

    expect(mocks.listPhotos).toHaveBeenNthCalledWith(
      1,
      "v1",
      expect.objectContaining({ limit: 7, before: undefined }),
    );
    expect(mocks.listPhotos).toHaveBeenNthCalledWith(
      2,
      "v1",
      expect.objectContaining({ limit: 7, before: "photo-cursor" }),
    );
    await waitFor(() => expect(photos.result.current.hasNextPage).toBe(false));
  });

  it("loads successive voyage pages using the opaque cursor", async () => {
    const { wrapper } = setup();
    mocks.listVoyages
      .mockResolvedValueOnce({ voyages: [], next_before: "cursor-2" })
      .mockResolvedValueOnce({ voyages: [], next_before: null });
    const hook = renderHook(() => useVoyages({ mine: true, limit: 5 }), { wrapper });
    await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
    expect(hook.result.current.hasNextPage).toBe(true);

    await act(async () => {
      await hook.result.current.fetchNextPage();
    });

    expect(mocks.listVoyages).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ mine: true, limit: 5, before: undefined }),
    );
    expect(mocks.listVoyages).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ mine: true, limit: 5, before: "cursor-2" }),
    );
    await waitFor(() => expect(hook.result.current.hasNextPage).toBe(false));
  });
});

describe("mutation cache effects", () => {
  it("seeds updated voyage data and invalidates its lists and map", async () => {
    const { client, wrapper } = setup();
    const set = vi.spyOn(client, "setQueryData");
    const invalidate = vi.spyOn(client, "invalidateQueries").mockResolvedValue(undefined);
    mocks.updateVoyage.mockResolvedValue(voyage);
    const hook = renderHook(() => useUpdateVoyage("v1"), { wrapper });

    await act(async () => {
      await hook.result.current.mutateAsync({ title: "Trip" });
    });

    expect(set).toHaveBeenCalledWith(keys.voyages.detail("v1"), voyage);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: keys.voyages.lists() });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: keys.voyages.map("v1") });
  });

  it("applies success invalidations for every write hook", async () => {
    const { client, wrapper } = setup();
    const invalidate = vi.spyOn(client, "invalidateQueries").mockResolvedValue(undefined);
    const remove = vi.spyOn(client, "removeQueries");
    const set = vi.spyOn(client, "setQueryData");
    mocks.createVoyage.mockResolvedValue(voyage);
    mocks.deleteVoyage.mockResolvedValue(undefined);
    mocks.detachPhoto.mockResolvedValue(undefined);
    mocks.reorderPhotos.mockResolvedValue(voyage);
    mocks.uploadPhotos.mockResolvedValue({ photos: [] });
    mocks.deletePhoto.mockResolvedValue(undefined);
    mocks.signOut.mockResolvedValue(undefined);

    const create = renderHook(() => useCreateVoyage(), { wrapper });
    const removeVoyage = renderHook(() => useDeleteVoyage(), { wrapper });
    const detach = renderHook(() => useDetachPhoto("v1"), { wrapper });
    const reorder = renderHook(() => useReorderPhotos("v1"), { wrapper });
    const upload = renderHook(() => useUploadPhotos(), { wrapper });
    const removePhoto = renderHook(() => useDeletePhoto(), { wrapper });
    const signout = renderHook(() => useSignOut(), { wrapper });

    await act(async () => {
      await create.result.current.mutateAsync({});
      await removeVoyage.result.current.mutateAsync("v1");
      await detach.result.current.mutateAsync("p1");
      await reorder.result.current.mutateAsync(["p1"]);
      await upload.result.current.mutateAsync({ voyageId: "v1", files: [] });
      await removePhoto.result.current.mutateAsync("p1");
      await signout.result.current.mutateAsync(undefined);
    });

    expect(mocks.createVoyage).toHaveBeenCalledWith({});
    expect(mocks.detachPhoto).toHaveBeenCalledWith("v1", "p1");
    expect(mocks.reorderPhotos).toHaveBeenCalledWith("v1", ["p1"]);
    expect(mocks.uploadPhotos).toHaveBeenCalledWith([], { voyageId: "v1" });
    expect(mocks.deletePhoto).toHaveBeenCalledWith("p1");
    expect(remove).toHaveBeenCalledWith({ queryKey: keys.voyages.detail("v1") });
    expect(remove).toHaveBeenCalledWith({ queryKey: keys.voyages.map("v1") });
    expect(remove).toHaveBeenCalledWith({ queryKey: keys.photos.detail("p1") });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: keys.photos.all });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: keys.voyages.all });
    expect(set).toHaveBeenCalledWith(keys.me, null);
  });

  it("does not retry a failed upload mutation", async () => {
    const { wrapper } = setup();
    mocks.uploadPhotos.mockRejectedValue(new Error("rejected"));
    const hook = renderHook(() => useUploadPhotos(), { wrapper });
    await act(async () => {
      await expect(hook.result.current.mutateAsync({ voyageId: "v1", files: [] })).rejects.toThrow("rejected");
    });
    expect(mocks.uploadPhotos).toHaveBeenCalledOnce();
  });
});
