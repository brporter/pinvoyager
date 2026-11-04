/// <reference types="node" />
import { readFileSync } from "node:fs";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Brand } from "../Brand";

const read = (path: string) => readFileSync(new URL(path, import.meta.url));
const html = () =>
  new DOMParser().parseFromString(read("../../../index.html").toString(), "text/html");

describe("shared iOS and website identity", () => {
  it("shows the light icon and selects the dark artwork for dark appearance", () => {
    render(
      <MemoryRouter>
        <Brand />
      </MemoryRouter>,
    );
    const brand = screen.getByRole("link", { name: "PinVoyager" });
    expect(brand).toHaveAttribute("href", "/");
    const image = brand.querySelector("picture img");
    expect(image).toHaveAttribute("src", "/brand/pinvoyager-light.png");
    expect(image).toHaveAttribute("alt", "");
    expect(image).toHaveAttribute("width", "32");
    expect(image).toHaveAttribute("height", "32");
    expect(brand.querySelector('source[media="(prefers-color-scheme: dark)"]')).toHaveAttribute(
      "srcset",
      "/brand/pinvoyager-dark.png",
    );
  });

  it.each(["light", "dark"])("provides a 64px PNG for the %s header and favicon", (appearance) => {
    const png = read(`../../../public/brand/pinvoyager-${appearance}.png`);
    expect(png.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([64, 64]);
    expect(png.length).toBeLessThan(25000);
    const icon = html().querySelector(
      `link[rel="icon"][href="/brand/pinvoyager-${appearance}.png"]`,
    );
    expect(icon?.getAttribute("type")).toBe("image/png");
    expect(icon?.getAttribute("sizes")).toBe("64x64");
    expect(icon?.getAttribute("media")).toBe(
      appearance === "dark" ? "(prefers-color-scheme: dark)" : null,
    );
  });

  it("ships distinct appearances and a matching 180px Apple touch icon", () => {
    expect(read("../../../public/brand/pinvoyager-light.png")).not.toEqual(
      read("../../../public/brand/pinvoyager-dark.png"),
    );
    const icon = read("../../../public/apple-touch-icon.png");
    expect([icon.readUInt32BE(16), icon.readUInt32BE(20)]).toEqual([180, 180]);
    expect(html().querySelector('link[rel="apple-touch-icon"]')?.getAttribute("href")).toBe(
      "/apple-touch-icon.png",
    );
  });
});
