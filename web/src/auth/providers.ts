/**
 * Provider names, for the copy that has to say one.
 *
 * The buttons themselves are `ProviderButton` in `src/components`, which owns
 * the mandated marks and the "Continue with X" grammar. What is left over is
 * the callback screen's prose — "Verifying your Google account", "That Apple ID
 * is already in use" — which needs a human name for a provider that arrived as
 * a string in a URL.
 */

import type { Provider } from "../components/ProviderButton";

const PROVIDER_NAME: Record<Provider, string> = {
  google: "Google",
  apple: "Apple",
  microsoft: "Microsoft",
  facebook: "Facebook",
};

/**
 * The display name for a provider named in a URL, or `null`.
 *
 * Looked up rather than interpolated. The value comes from the address bar and
 * ends up inside a sentence, so an unrecognised one produces copy with no
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
