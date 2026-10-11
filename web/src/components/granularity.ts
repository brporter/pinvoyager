import type { Granularity } from "../api/types";
export const GRANULARITIES: readonly Granularity[] = ["exact", "km1", "km10", "km50"];

/** Fallback labels. Prefer the server's `granularity_label` where you have a
 *  voyage to hand — it is rendered rather than derived so the several places it
 *  appears cannot disagree. */
export const GRANULARITY_LABEL: Record<Granularity, string> = {
  exact: "Exact",
  km1: "~1 km",
  km10: "~10 km",
  km50: "~50 km",
};

/** Per-axis half-width of the public rounding cell, in meters. */
export const GRANULARITY_PRECISION: Record<Granularity, number | null> = {
  exact: null,
  km1: 500,
  km10: 5000,
  km50: 25000,
};
