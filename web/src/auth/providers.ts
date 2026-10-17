/** Provider display names used in callback messages. */

import type { Provider } from "../components/ProviderButton";

const PROVIDER_NAME: Record<Provider, string> = {
  google: "Google",
  apple: "Apple",
  microsoft: "Microsoft",
};

/**
 * The display name for a provider named in a URL, or `null`.
 *
 * Looked up rather than interpolated. The value comes from the address bar and
 * ends up inside a sentence, so an unrecognized one produces copy with no
 * provider name in it — "Verifying your account" — instead of rendering
 * whatever the caller wrote.
 */
export function providerNameOf(raw: string | null | undefined): string | null {
  if (raw === null || raw === undefined) {
    return null;
  }

  const key = raw.toLowerCase();
  return Object.hasOwn(PROVIDER_NAME, key) ? PROVIDER_NAME[key as Provider] : null;
}

/** The provider itself, when a control needs to restart its flow. */
export function providerOf(raw: string | null | undefined): Provider | null {
  if (raw === null || raw === undefined) {
    return null;
  }

  const key = raw.toLowerCase();
  return Object.hasOwn(PROVIDER_NAME, key) ? (key as Provider) : null;
}
