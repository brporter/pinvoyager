/**
 * Loads the voyage map on demand so other screens do not load MapLibre.
 * The Suspense fallback reserves the map's space while its chunk loads.
 */

import { Suspense, lazy } from "react";

import type { VoyageMapProps } from "./VoyageMap";

const VoyageMap = lazy(() => import("./VoyageMap"));

export function LazyVoyageMap(props: VoyageMapProps) {
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
      <VoyageMap {...props} />
    </Suspense>
  );
}
