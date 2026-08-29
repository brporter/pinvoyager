/**
 * Validation of caller-supplied post-sign-in destinations.
 *
 * This is the client half of the open-redirect check in
 * `crates/api/src/http/auth.rs::safe_redirect`. The server is the authority —
 * it validates a `redirect_to` before storing it and again before using it, so
 * nothing here is load-bearing for security. What this buys is agreement: the
 * client never sends a destination the server would silently drop, so "sign in
 * and come back to where I was" either works or is visibly replaced by the
 * default, rather than half-working.
 *
 * The rules are deliberately transcribed rather than loosened. A value that
 * passes here and fails there would strand the user at `/` with no explanation,
 * and a value that passes there and fails here would be a needless downgrade.
 *
 * The check rejects rather than repairs, for the same reason the server does:
 * normalizing a hostile value is where the interesting bypasses live.
 */

/**
 * Mirror of `ALLOWED_REDIRECTS`. An entry matches its own path exactly, or any
 * path beneath it — `/voyages` covers `/voyages/{id}/edit`.
 *
 * `/u/{userId}` is deliberately absent, because it is absent on the server too.
 * A profile is public, so nothing ever needs to sign in to reach one.
 */
const ALLOWED_REDIRECTS = ["/", "/discover", "/settings", "/upload", "/voyages"] as const;

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
    const code = character.codePointAt(0) ?? 0;
    if (code < 0x20 || code >= 0x7f) {
      return null;
    }
  }

  // Only the path is matched. A query string or fragment is the application's
  // business; the origin is settled by the checks above plus the allowlist.
  const path = candidate.split(/[?#]/, 1)[0] ?? DEFAULT_DESTINATION;

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
