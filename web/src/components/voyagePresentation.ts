import { isApiError } from "../api/client";
import type { VoyageList, VoyageSummary } from "../api/types";

/** Every voyage across every page fetched so far. */
export function voyagesOf(pages: { pages: VoyageList[] } | undefined): VoyageSummary[] {
  return pages?.pages.flatMap((page) => page.voyages) ?? [];
}

/** Formats publication time as a localized month and year. */
export function monthYear(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

/** The `x-request-id` behind a failure, for the error screen to quote. Null
 *  when the failure never reached the service. */
export function referenceOf(error: unknown): string | null {
  return isApiError(error) ? error.requestId : null;
}
