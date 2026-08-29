/**
 * Tests for the stored default location precision.
 *
 * This value decides how precisely a *new* voyage publishes its coordinates,
 * for a user who never opens the precision control at all. That makes the
 * fallback a privacy decision rather than a formatting one: if an unreadable or
 * absent stored value resolved to `exact`, a user who had never touched the
 * setting would be publishing their exact positions because of a corrupted
 * localStorage entry.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { readDefaultGranularity } from "../Settings";

const KEY = "pinvoyager.default_granularity";

/** The setting a user gets when nothing usable is stored. Deliberately not
 *  `exact`. */
const CAUTIOUS_DEFAULT = "km1";

afterEach(() => {
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("readDefaultGranularity", () => {
  it("returns a stored value that is one of the four settings", () => {
    window.localStorage.setItem(KEY, "km50");
    expect(readDefaultGranularity()).toBe("km50");
  });

  it("returns exact when the user has genuinely chosen it", () => {
    window.localStorage.setItem(KEY, "exact");
    expect(readDefaultGranularity()).toBe("exact");
  });

  it("falls back to a coarse setting when nothing is stored", () => {
    expect(readDefaultGranularity()).toBe(CAUTIOUS_DEFAULT);
  });

  /** The important one. A value that is not a recognized setting — an older
   *  spelling, a truncated write, something another script put there — must
   *  not be treated as permission to publish exact coordinates. */
  it("falls back to a coarse setting rather than exact when the stored value is unrecognized", () => {
    for (const junk of ["", "EXACT", "km5", "true", "null", "1km", "{}"]) {
      window.localStorage.setItem(KEY, junk);
      const resolved = readDefaultGranularity();

      expect(
        resolved,
        `stored value ${JSON.stringify(junk)} must not resolve to exact`,
      ).toBe(CAUTIOUS_DEFAULT);
    }
  });

  /** Safari in a private window throws on localStorage access rather than
   *  returning null. Reading the setting must not take the settings page down
   *  with it, and must still fail toward the cautious answer. */
  it("falls back to a coarse setting when storage throws instead of returning null", () => {
    vi.spyOn(window.localStorage, "getItem").mockImplementation(() => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    });

    expect(() => readDefaultGranularity()).not.toThrow();
    expect(readDefaultGranularity()).toBe(CAUTIOUS_DEFAULT);
  });
});
