/** Load MapLibre only when the upload flow reaches review. */
import { Suspense, lazy } from "react";

import type { UploadReviewMapProps } from "./UploadReviewMap";

const UploadReviewMap = lazy(() => import("./UploadReviewMap"));

export function LazyUploadReviewMap(props: UploadReviewMapProps) {
  return (
    <Suspense
      fallback={
        <div
          className="pv-map"
          style={props.style}
          role="region"
          aria-label={props.label}
          aria-busy="true"
        >
          <span className="pv-sr">Loading the map…</span>
        </div>
      }
    >
      <UploadReviewMap {...props} />
    </Suspense>
  );
}
