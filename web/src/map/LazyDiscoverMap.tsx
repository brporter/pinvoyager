/**
 * Loads the discovery map on demand. Suspense shows a map placeholder
 * while the component and shared MapLibre chunk load.
 */

import { Suspense, lazy } from "react";

import type { DiscoverMapProps } from "./DiscoverMap";

const DiscoverMap = lazy(() => import("./DiscoverMap"));

export function LazyDiscoverMap(props: DiscoverMapProps) {
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
      <DiscoverMap {...props} />
    </Suspense>
  );
}
