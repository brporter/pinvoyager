/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

describe("site favicon", () => {
  it("uses the Horizon Pin artwork", () => {
    expect(read("../../../public/favicon.svg")).toBe(
      read("../../../../design/logos/horizon-pin.svg"),
    );
  });

  it("declares the SVG favicon for every application route", () => {
    const html = new DOMParser().parseFromString(read("../../../index.html"), "text/html");
    expect(html.querySelector('link[rel="icon"]')?.getAttribute("href")).toBe("/favicon.svg");
    expect(html.querySelector('link[rel="icon"]')?.getAttribute("type")).toBe("image/svg+xml");
  });
});
