/**
 * Where the navigation points, signed in and signed out.
 *
 * The signed-out half is the interesting one. Every published voyage is
 * world-readable, so a signed-out visitor is an ordinary user of this site and
 * the tab bar stays — hiding three quarters of it makes the product look
 * smaller than it is and hides where signing in would get them. What the tabs
 * must not do is point at a route that will bounce them, so each one carries
 * its own destination into the sign-in screen.
 */

import { describe, expect, it } from "vitest";

import type { CurrentUser } from "../../api/types";
import { tabDestinations, topNavLinks } from "../destinations";

const USER: CurrentUser = {
  id: "0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
  display_name: "Ada Lovelace",
  avatar_url: null,
  is_site_admin: false,
  created_at: "2026-02-01T10:00:00Z",
};

describe("tabDestinations", () => {
  it("keeps all four tabs when signed out and routes three through sign-in", () => {
    const tabs = tabDestinations(null);

    expect(tabs.map((tab) => tab.label)).toEqual(["Discover", "Voyages", "Upload", "Profile"]);
    expect(tabs.map((tab) => tab.to)).toEqual([
      // Public, and stays public.
      "/",
      // No destination: it depends on an account id that is not known yet, and
      // the server's allowlist refuses `/u` anyway.
      "/signin",
      "/signin?redirect_to=%2Fupload",
      "/signin?redirect_to=%2Fsettings",
    ]);
  });

  it("points at the real destinations when signed in", () => {
    expect(tabDestinations(USER).map((tab) => tab.to)).toEqual([
      "/",
      `/u/${USER.id}`,
      "/upload",
      "/settings",
    ]);
  });

  it("matches Discover only on the exact path", () => {
    // Without this every route would light the first tab, because `/` is a
    // prefix of all of them.
    const [discover] = tabDestinations(null);
    expect(discover?.end).toBe(true);
  });
});

describe("topNavLinks", () => {
  it("drops the signed-in destination when there is nothing behind it", () => {
    expect(topNavLinks(null).map((link) => link.label)).toEqual(["Discover"]);
  });

  it("adds My voyages when signed in", () => {
    expect(topNavLinks(USER)).toEqual([
      { key: "discover", label: "Discover", to: "/", end: true },
      { key: "voyages", label: "My voyages", to: `/u/${USER.id}`, end: false },
    ]);
  });
});
