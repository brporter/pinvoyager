/** Keep MapLibre out of the initial application bundle. */
import { Suspense, lazy } from "react";
import type { PrecisionPreviewProps } from "./PrecisionPreview";

const PrecisionPreview = lazy(() => import("./PrecisionPreview"));

export function LazyPrecisionPreview(props: PrecisionPreviewProps) {
  return (
    <Suspense
      fallback={
        <div
          className="pv-map pv-gran-preview"
          style={{ height: 280 }}
          role="region"
          aria-label="Location precision example — St. Louis, Missouri"
          aria-busy="true"
        >
          <span className="pv-sr">Loading the map…</span>
        </div>
      }
    >
      <PrecisionPreview {...props} />
    </Suspense>
  );
}
