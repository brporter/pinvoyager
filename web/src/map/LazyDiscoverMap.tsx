/**
 * The discovery map, loaded only when the discovery screen is on screen.
 *
 * Same reasoning as `LazyVoyageMap.tsx`: MapLibre is a substantial download and
 * every module reachable from the entry chunk is fetched, parsed and executed
 * before first paint. The discovery feed is the application's front door, so it
 * is the screen where that would cost the most — the voyage grid must not wait
 * for a mapping library to arrive before it can be read.
 *
 * Vite puts MapLibre in a chunk shared by this boundary and the voyage one, so
 * moving from discovery to a voyage does not download it twice.
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
