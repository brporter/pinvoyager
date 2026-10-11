import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { TraceContext } from "../../api/client";
import { useUploadPhotos } from "../../api/hooks";
import { reasonFor, type Item, type ItemState } from "./model";

/** Owns the batch lifetime, per-file cancellation, progress, and retry identity. */
export function useUploadBatch(destination: RefObject<string | null>, traceContext: TraceContext) {
  const [items, setItems] = useState<Item[]>([]);

  /** One controller per in-flight file, so a single row's cancel button
   *  abandons that upload and nothing else. */
  const controllers = useRef(new Map<string, AbortController>());
  const batchController = useRef(new AbortController());

  useEffect(() => {
    const inFlight = controllers.current;
    batchController.current = new AbortController();
    return () => {
      batchController.current.abort();
      for (const controller of inFlight.values()) controller.abort();
    };
  }, []);

  const patch = useCallback((id: string, change: ItemState) => {
    setItems((current) =>
      current.map((item) => (item.id === id ? { id: item.id, file: item.file, ...change } : item)),
    );
  }, []);

  const { mutateAsync: send } = useUploadPhotos();

  const uploadOne = useCallback(
    async (item: Item): Promise<void> => {
      const controller = new AbortController();
      controllers.current.set(item.id, controller);
      patch(item.id, { status: "uploading", progress: 0, failure: undefined });

      try {
        const result = await send({
          voyageId: destination.current!,
          traceContext,
          files: [item.file],
          idempotencyKey: item.id,
          signal: controller.signal,
          onProgress: (fraction) => {
            if (fraction === null) {
              patch(item.id, { status: "processing", progress: 100 });
              return;
            }
            const percent = Math.round(fraction * 100);
            // The bytes are gone but the response has not come back: the
            // server is decoding the header and reading EXIF. The design draws
            // that as its own state — "reading location data" — rather than a
            // bar that sits at 100% looking stuck.
            patch(
              item.id,
              percent >= 100
                ? { status: "processing", progress: 100 }
                : { status: "uploading", progress: percent },
            );
          },
        });

        const photo = result.photos[0];
        if (photo === undefined) {
          patch(item.id, {
            status: "error",
            progress: 0,
            failure: {
              message: "The server accepted the file but returned no photo",
              retryable: true,
            },
          });
          return;
        }

        patch(item.id, {
          status: "uploaded",
          progress: 100,
          photo,
          failure: undefined,
        });
      } catch (error) {
        patch(item.id, {
          status: "error",
          progress: 0,
          failure: controller.signal.aborted
            ? { message: "Canceled", retryable: true }
            : reasonFor(error, item.file),
        });
      } finally {
        controllers.current.delete(item.id);
      }
    },
    [patch, send, destination, traceContext],
  );

  /** Sequential on purpose. Six parallel uploads race the per-user rate
   *  limiter into a 429 that has nothing to do with the files, and the design
   *  draws one row in flight at a time. */
  const runBatch = useCallback(
    async (batch: readonly Item[]): Promise<void> => {
      const signal = batchController.current.signal;
      for (const item of batch) {
        if (signal.aborted) break;
        await uploadOne(item);
      }
    },
    [uploadOne],
  );

  const add = useCallback(
    (files: File[]) => {
      const batch: Item[] = files.map((file) => ({
        id: crypto.randomUUID(),
        file,
        status: "queued",
        progress: 0,
      }));
      setItems((current) => [...current, ...batch]);
      void runBatch(batch);
    },
    [runBatch],
  );
  return {
    items,
    add,
    cancel: (id: string) => controllers.current.get(id)?.abort(),
    retry: (id: string) => {
      const item = items.find((item) => item.id === id);
      if (item) void uploadOne(item);
    },
    remove: (id: string) => setItems((current) => current.filter((item) => item.id !== id)),
    reset: () => setItems([]),
  };
}
