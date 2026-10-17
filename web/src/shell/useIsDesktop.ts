/**
 * Subscribes to the 768px media query to choose desktop or mobile navigation.
 * Only the selected navigation is mounted. Returns the mobile layout when
 * matchMedia is unavailable.
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

/** Returns true at widths of at least 768px, or false without matchMedia. */
export function useIsDesktop(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, () => false);
}
