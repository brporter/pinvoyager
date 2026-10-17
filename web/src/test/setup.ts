// Test environment setup, loaded by vitest before every suite.
import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

/**
 * Replaces MapLibre with the shared test double for jsdom suites.
 * The double provides DOM markers and camera events without WebGL, drawing,
 * or tile requests. Projection tests can use the real library separately.
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
