import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router";

import fixture from "../../api/__fixtures__/voyage_detail.json";
import photoFixture from "../../api/__fixtures__/photo.json";
import { AuthContext } from "../../auth/context";
import { Upload } from "../Upload";
import { VoyageDetail } from "../VoyageDetail";
import { ADA, authState } from "./support";

// Keep real API clients and hooks; substitute only the HTTP transports.
class UploadXhr extends EventTarget {
  static requests: { url: string; files: File[] }[] = [];
  upload = new EventTarget();
  withCredentials = false;
  status = 201;
  statusText = "Created";
  responseText = "";
  url = "";
  open(_method: string, url: string) { this.url = url; }
  setRequestHeader() {}
  getResponseHeader() { return null; }
  send(form: FormData) {
    UploadXhr.requests.push({ url: this.url, files: form.getAll("files") as File[] });
    this.responseText = JSON.stringify({ photos: [{ ...photoFixture, id: fixture.photos[UploadXhr.requests.length - 1].id, variant_status: "pending" }] });
    queueMicrotask(() => this.dispatchEvent(new Event("load")));
  }
  abort() { this.dispatchEvent(new Event("abort")); }
}

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it("shows every uploaded photo on the first draft visit as variants finish separately", async () => {
  await import("../../map/VoyageMap");
  vi.useFakeTimers();
  UploadXhr.requests = [];
  vi.stubGlobal("XMLHttpRequest", UploadXhr);
  const draft = { ...fixture, id: "v1", owner_id: ADA.id, published: false, published_at: null, placed_count: 3, unplaced_count: 0,
    photos: fixture.photos.map(photo => ({ ...photo, placed: true, coordinate: fixture.photos[1].coordinate })) };
  let reads = 0;
  const fetchMock = vi.fn(async (url: string, options?: RequestInit) => {
    if (url === "/api/voyages" && options?.method === "POST") {
      expect(JSON.parse(options.body as string)).toEqual({ title: "Winter trip", description: "Snow and ferries" });
      expect(UploadXhr.requests).toHaveLength(0);
      return new Response(JSON.stringify({ ...draft, photos: [] }), { status: 201 });
    }
    if (url === "/api/voyages/v1") {
      reads += 1;
      return new Response(JSON.stringify({ ...draft, photos: draft.photos.map((photo, index) => ({
        ...photo, variant_status: index < reads ? "ready" : "pending",
      })) }));
    }
    throw new Error(`Unexpected HTTP request: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(<QueryClientProvider client={client}><AuthContext.Provider value={authState(ADA)}>
    <MemoryRouter initialEntries={["/upload"]}><Routes>
      <Route path="/upload" element={<Upload />} />
      <Route path="/voyages/:id" element={<VoyageDetail />} />
    </Routes></MemoryRouter>
  </AuthContext.Provider></QueryClientProvider>);
  fireEvent.change(screen.getByRole("textbox", { name: /Voyage Name/ }), { target: { value: "Winter trip" } });
  fireEvent.change(screen.getByRole("textbox", { name: /Voyage Description/ }), { target: { value: "Snow and ferries" } });
  fireEvent.click(screen.getByRole("button", { name: "Next: upload photos" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  const files = ["one.jpg", "two.jpg", "three.jpg"].map(name => new File([name], name, { type: "image/jpeg" }));
  fireEvent.change(view.container.querySelector<HTMLInputElement>('input[type="file"]')!, { target: { files } });
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  expect(UploadXhr.requests.map(request => request.files[0].name)).toEqual(files.map(file => file.name));
  expect(UploadXhr.requests.every(request => request.url === "/api/voyages/v1/photos")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Review 3 photos" }));
  fireEvent.click(screen.getByRole("button", { name: "View voyage" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  expect(screen.getByRole("button", { name: "Publish voyage" })).toBeInTheDocument();
  const renderedImages = () => view.container.querySelectorAll(".pv-photo img");
  expect(renderedImages()).toHaveLength(1);
  expect(view.container.querySelectorAll(".pv-photo.pv-skeleton")).toHaveLength(2);
  fireEvent.click(view.container.querySelector<HTMLButtonElement>("button.pv-cluster")!);
  const gallery = screen.getByRole("dialog", { name: "Photos at this location" });
  expect(within(gallery).getAllByRole("img")).toHaveLength(1);
  fireEvent.click(within(gallery).getByRole("button", { name: "Next photo" }));
  expect(within(gallery).getByText("Processing photo…")).toBeInTheDocument();
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(within(gallery).getByRole("img")).toHaveAttribute("src", draft.photos[1].display_url);
  fireEvent.click(within(gallery).getByRole("button", { name: "Next photo" }));
  expect(within(gallery).getByText("Processing photo…")).toBeInTheDocument();
  expect(renderedImages()).toHaveLength(2);
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(renderedImages()).toHaveLength(3);
  expect(within(gallery).getByRole("img")).toHaveAttribute("src", draft.photos[2].display_url);
  expect(within(gallery).queryByText("Processing photo…")).not.toBeInTheDocument();
  expect(Array.from(renderedImages(), image => image.getAttribute("src"))).toEqual(draft.photos.map(photo => photo.thumbnail_url));
  expect(view.container.querySelectorAll(".pv-photo.pv-skeleton")).toHaveLength(0);
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(reads).toBe(3);
  view.unmount(); client.clear();
});
