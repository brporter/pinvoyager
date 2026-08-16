/**
 * The client half of the open-redirect check.
 *
 * These cases are the same ones `crates/api/src/http/auth.rs` tests, on
 * purpose. The server is the authority and re-validates everything, so what is
 * being protected here is *agreement*: a value the client sends and the server
 * drops strands the user at `/` for no visible reason, and a value the client
 * drops and the server would have kept is a needless downgrade. Two lists that
 * are meant to be identical stay identical only if both are tested.
 */

import { describe, expect, it } from "vitest";

import { DEFAULT_DESTINATION, destinationOr, safeRedirect, signInPath } from "../redirect";

describe("safeRedirect", () => {
  it("refuses every way of naming another origin", () => {
    const hostile = [
      "https://evil.example/",
      // Protocol-relative: a browser reads this as another origin despite the
      // leading slash.
      "//evil.example/",
      // And these, because a browser folds backslashes to forward slashes
      // while a string comparison does not.
      "/\\evil.example",
      "/\\/evil.example",
      "http:/evil.example",
      "evil.example",
      // Control characters, which become header splitting once the value
      // reaches the server's `Location`.
      "/discover\r\nSet-Cookie: a=b",
      "/discover\nX: y",
      // On-site is not by itself enough. An unrecognised path is a typo or a
      // probe, not a destination.
      "/admin",
      // Prefix matches without a separator — the reason the check looks at the
      // character after the match rather than using `startsWith` alone.
      "/settings-evil.example",
      "/voyagesomething",
    ];

    for (const candidate of hostile) {
      expect(safeRedirect(candidate), `accepted ${JSON.stringify(candidate)}`).toBeNull();
    }
  });

  it("keeps a recognised on-site path verbatim", () => {
    const allowed = [
      "/",
      "/discover",
      "/settings",
      "/upload",
      "/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
      // The three protected routes must all survive the round trip, or signing
      // in from one of them never returns to it.
      "/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f/edit",
      "/discover?near=me#top",
    ];

    for (const candidate of allowed) {
      expect(safeRedirect(candidate), `refused ${JSON.stringify(candidate)}`).toBe(candidate);
    }
  });

  it("refuses an over-long destination", () => {
    expect(safeRedirect(`/discover?q=${"a".repeat(512)}`)).toBeNull();
  });

  it("refuses nothing at all", () => {
    expect(safeRedirect(null)).toBeNull();
    expect(safeRedirect(undefined)).toBeNull();
    expect(safeRedirect("")).toBeNull();
  });
});

describe("destinationOr", () => {
  it("substitutes the default for a refusal rather than propagating null", () => {
    expect(destinationOr("https://evil.example/")).toBe(DEFAULT_DESTINATION);
    expect(destinationOr(null)).toBe(DEFAULT_DESTINATION);
    expect(destinationOr("/settings")).toBe("/settings");
    expect(destinationOr(null, "/upload")).toBe("/upload");
  });
});

describe("signInPath", () => {
  it("carries a validated destination and encodes it", () => {
    expect(signInPath("/voyages/abc/edit")).toBe(
      "/signin?redirect_to=%2Fvoyages%2Fabc%2Fedit",
    );
  });

  it("drops a destination that does not pass instead of forwarding it", () => {
    // The important half. A `redirect_to` that reached the sign-in screen
    // unchecked would make a link to this site a credible first hop for a
    // phishing flow — the address bar shows our domain right up to the moment
    // the browser leaves.
    expect(signInPath("https://evil.example/")).toBe("/signin");
    expect(signInPath("//evil.example/")).toBe("/signin");
    expect(signInPath(null)).toBe("/signin");
  });
});
