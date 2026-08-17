/**
 * The map, loaded only when a screen actually shows one.
 *
 * This is target 12. MapLibre plus its stylesheet is a substantial download,
 * and every module reachable from the entry chunk is downloaded, parsed and
 * executed before first paint — including on the discovery feed, the upload
 * screen and the settings screen, none of which draw a map. The dynamic
 * `import()` below is what puts it in its own chunk instead.
 *
 * The fallback is the design system's own `.pv-map` surface at the exact
 * dimensions the map will occupy, so the layout does not shift when the real
 * map arrives — the box is already the right size and the tiles fill it in.
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
