/**
 * Validates local post-sign-in destinations using the server's path rules.
 * Rejected values fall back to the default route. The server independently
 * validates destinations before storing or redirecting to them.
 */

/**
 * Allowed paths and their descendants, matching the server allowlist.
 * A following slash is required for descendant matches.
 */
const ALLOWED_REDIRECTS = ["/", "/discover", "/settings", "/upload", "/voyages", "/admin"] as const;

/** Mirror of `MAX_REDIRECT_LEN`. */
const MAX_REDIRECT_LEN = 512;

/** Where a completed sign-in lands when nothing better was asked for. */
export const DEFAULT_DESTINATION = "/";

/** The query parameter both halves of the flow use to carry a destination. */
export const REDIRECT_PARAM = "redirect_to";

/**
 * Returns `candidate` unchanged when it names a path on this site that we
 * recognize, and `null` otherwise.
 */
export function safeRedirect(candidate: string | null | undefined): string | null {
  if (candidate === null || candidate === undefined) {
    return null;
  }

  if (candidate.length > MAX_REDIRECT_LEN || !candidate.startsWith("/")) {
    return null;
  }

  // `//evil.example` and `/\evil.example` are protocol-relative URLs: a browser
  // reads both as another origin despite the leading slash, and the second only
  // because it folds backslashes to forward slashes while a string comparison
  // does not.
  if (candidate.startsWith("//") || candidate.includes("\\")) {
    return null;
  }

  // Printable ASCII only. Excludes the CR and LF that would be a header-
  // splitting attempt once this reaches the server, and non-ASCII bytes, which
  // have to be percent-encoded to be legal in a URL anyway.
  for (const character of candidate) {
    // String iteration yields non-empty code-point strings, so index zero is
    // always present (including for astral characters such as emoji).
    const code = character.codePointAt(0)!;
    if (code < 0x20 || code >= 0x7f) {
      return null;
    }
  }

  // Only the path is matched. A query string or fragment is the application's
  // business; the origin is settled by the checks above plus the allowlist.
  // `split` always returns at least one entry, including for an empty string.
  const path = candidate.split(/[?#]/, 1)[0]!;

  const allowed = ALLOWED_REDIRECTS.some(
    (prefix) =>
      path === prefix ||
      // A path *beneath* an entry. The explicit separator check is what stops
      // `/settings-evil.example` from passing on the strength of `/settings`.
      (path.startsWith(prefix) && path[prefix.length] === "/"),
  );

  return allowed ? candidate : null;
}

/** `safeRedirect` with the default substituted for a refusal. */
export function destinationOr(
  candidate: string | null | undefined,
  fallback: string = DEFAULT_DESTINATION,
): string {
  return safeRedirect(candidate) ?? fallback;
}

/**
 * The sign-in URL that returns to `destination` afterwards.
 *
 * The destination is validated here rather than trusted, because the callers
 * are places like the tab bar and the route guard which build it from the
 * current location — and the current location is whatever the address bar says.
 */
export function signInPath(destination?: string | null): string {
  const safe = safeRedirect(destination);
  if (safe === null) {
    return "/signin";
  }
  return `/signin?${REDIRECT_PARAM}=${encodeURIComponent(safe)}`;
}
