/**
 * Whether the user has asked for reduced motion.
 *
 * The design system already collapses CSS transitions under
 * `@media (prefers-reduced-motion: reduce)`, so most of the application needs
 * nothing. Two things sit outside CSS and have to read the preference
 * themselves:
 *
 *  * **MapLibre's camera.** Flights, eases and zooms are animated by MapLibre
 *    in JavaScript against its own clock. A media query cannot reach them, so
 *    the preference has to be passed into each camera call as a duration of
 *    zero. `design/components/map-primitives.html` says so explicitly under
 *    "Notes for phase 7", and without it the map keeps flying across continents
 *    for exactly the users who asked it not to.
 *  * **The mobile sheet's height.** It is animated by a CSS transition, but the
 *    screen also has to know to skip it, because a 300px slide is the motion
 *    being objected to.
 *
 * Read at the moment of use rather than subscribed to. These are one-shot
 * decisions taken as an animation starts; there is no ongoing state to keep in
 * sync, and a user who changes the setting mid-flight can wait for the next
 * one.
 */
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
