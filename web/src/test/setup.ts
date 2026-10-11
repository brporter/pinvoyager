// Test environment setup, loaded by vitest before every suite.
import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

/**
 * MapLibre is replaced everywhere, not per test file.
 *
 * It cannot run here under any circumstances: it needs a WebGL context and
 * jsdom does not implement one, so the real library throws inside its own
 * constructor. Mocking it once means a screen that grows a map later cannot
 * quietly start failing — or, worse, quietly start passing while the map inside
 * it never renders.
 *
 * There is a cost reason too. Any test file that reaches the code-split map
 * boundary pulls a 950 kB module through Vite's transform pipeline, once per
 * file. Before this was here the suite took minutes rather than seconds, for a
 * library none of it could use.
 *
 * The substitute is `web/__mocks__/maplibre-gl.ts`; see that file for what it
 * is faithful about.
 */
vi.mock("maplibre-gl");

// jsdom has no modal top layer or dialog form default action. Model only the
// native methods/events used by the gallery; browser focus trapping and image
// decoding still require browser verification.
HTMLDialogElement.prototype.showModal = vi.fn(function (this: HTMLDialogElement) {
  this.open = true;
  this.querySelector<HTMLElement>("button")?.focus();
});
HTMLDialogElement.prototype.close = vi.fn(function (this: HTMLDialogElement) {
  this.open = false;
  this.dispatchEvent(new Event("close"));
});
document.addEventListener("submit", (event) => {
  const form = event.target as HTMLFormElement;
  if (form.getAttribute("method") !== "dialog") return;
  event.preventDefault();
  form.closest("dialog")!.close();
});
