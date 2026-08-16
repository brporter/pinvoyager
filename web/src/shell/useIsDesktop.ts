/**
 * Which navigation chrome to render.
 *
 * This is a media query evaluated in JavaScript rather than in CSS, and that is
 * a deliberate trade. The top bar and the tab bar are not the same markup with
 * different styling — the tab bar is four links, the top bar is a brand, two
 * links, a button and an account menu — so the CSS approach would mean
 * rendering both and hiding one, which puts a duplicate set of navigation
 * landmarks and a duplicate set of focusable controls in the accessibility tree
 * on every route. `display: none` hides them from a screen reader but not from
 * every assistive technology, and it does not stop them existing in the DOM.
 *
 * The breakpoint is `md`, 768px, from `design/tokens.css`. It is written
 * literally here for the same reason the design system writes it literally in
 * media queries: it is a number, and the token is documentation of the number.
 */

import { useSyncExternalStore } from "react";

const DESKTOP_QUERY = "(min-width: 768px)";

function subscribe(onChange: () => void): () => void {
  // `matchMedia` is absent in some test environments and in any non-browser
  // renderer. Nothing to subscribe to there; `getSnapshot` returns the mobile
  // answer and it never changes.
  if (typeof window.matchMedia !== "function") {
    return () => {};
  }

  const list = window.matchMedia(DESKTOP_QUERY);
  list.addEventListener("change", onChange);
  return () => {
    list.removeEventListener("change", onChange);
  };
}

function getSnapshot(): boolean {
  if (typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia(DESKTOP_QUERY).matches;
}

/**
 * True at and above the `md` breakpoint.
 *
 * Defaults to false — the mobile shell — when the query cannot be evaluated.
 * Guessing mobile is the safe direction: the tab bar fits a desktop viewport,
 * whereas the desktop bar's contents do not fit 390px.
 */
export function useIsDesktop(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, () => false);
}
