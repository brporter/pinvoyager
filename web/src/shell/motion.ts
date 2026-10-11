/** Applies the reduced-motion preference to JavaScript camera transitions. */
export function prefersReducedMotion(): boolean {
  // Absent in jsdom and in any non-browser renderer. No preference expressed is
  // not the same as a preference for reduced motion, so the answer is false.
  if (typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * How long a camera movement should take, in milliseconds.
 *
 * Zero is what MapLibre reads as "jump straight there". Passing `animate:
 * false` would do the same, but every camera method takes `duration`, so one
 * number covers `easeTo`, `flyTo`, `fitBounds`, `zoomIn` and `zoomOut` without
 * each call site having to remember which option that method honors.
 */
export function cameraDuration(normalMs: number): number {
  return prefersReducedMotion() ? 0 : normalMs;
}
