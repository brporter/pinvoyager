import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiRequestError, UnauthorizedError, uploadPhotos } from "../client";

class MockXhr extends EventTarget {
  static instances: MockXhr[] = [];
  upload = new EventTarget();
  open = vi.fn();
  send = vi.fn();
  abort = vi.fn(() => this.dispatchEvent(new Event("abort")));
  setRequestHeader = vi.fn();
  getResponseHeader = vi.fn((name: string) => this.headers.get(name.toLowerCase()) ?? null);
  status = 200;
  statusText = "";
  responseText = "{}";
  withCredentials = false;
  headers = new Map<string, string>();

  constructor() {
    super();
    MockXhr.instances.push(this);
  }
}

function installXhr(): void {
  MockXhr.instances = [];
  vi.stubGlobal("XMLHttpRequest", MockXhr);
}

function lastXhr(): MockXhr {
  return MockXhr.instances.at(-1)!;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("uploadPhotos", () => {
  it("sends multipart files with credentials and reports known and unknown progress", async () => {
    installXhr();
    const onProgress = vi.fn();
    const file = new File(["image bytes"], "trip.jpg", { type: "image/jpeg" });
    const result = uploadPhotos([file], { onProgress });
    const xhr = lastXhr();

    expect(xhr.open).toHaveBeenCalledWith("POST", "/api/photos");
    expect(xhr.withCredentials).toBe(true);
    expect(xhr.setRequestHeader).toHaveBeenCalledWith("accept", "application/json");
    expect(xhr.setRequestHeader).not.toHaveBeenCalledWith("content-type", expect.any(String));
    const form = xhr.send.mock.calls[0][0] as FormData;
    expect(form.getAll("files")).toHaveLength(1);
    expect((form.get("files") as File).name).toBe("trip.jpg");

    const known = new Event("progress");
    Object.defineProperties(known, {
      lengthComputable: { value: true },
      loaded: { value: 25 },
      total: { value: 100 },
    });
    xhr.upload.dispatchEvent(known);
    const unknown = new Event("progress");
    Object.defineProperties(unknown, {
      lengthComputable: { value: false },
      loaded: { value: 10 },
      total: { value: 0 },
    });
    xhr.upload.dispatchEvent(unknown);
    expect(onProgress.mock.calls).toEqual([[0.25], [null]]);

    xhr.responseText = JSON.stringify({ photos: [] });
    xhr.dispatchEvent(new Event("load"));
    await expect(result).resolves.toEqual({ photos: [] });
  });

  it("rejects before creating a request when its signal is already aborted", async () => {
    installXhr();
    const controller = new AbortController();
    controller.abort("user canceled");

    await expect(uploadPhotos([], { signal: controller.signal })).rejects.toMatchObject({
      name: "NetworkError",
      cause: "user canceled",
    });
    expect(MockXhr.instances).toHaveLength(0);
  });

  it("aborts an in-flight request when its signal is aborted", async () => {
    installXhr();
    const controller = new AbortController();
    const result = uploadPhotos([], { signal: controller.signal });
    const xhr = lastXhr();
    controller.abort("navigation");

    expect(xhr.abort).toHaveBeenCalledOnce();
    await expect(result).rejects.toMatchObject({
      name: "NetworkError",
      cause: "navigation",
    });
  });

  it("reports XHR transport errors", async () => {
    installXhr();
    const result = uploadPhotos([]);
    lastXhr().dispatchEvent(new Event("error"));

    await expect(result).rejects.toMatchObject({
      name: "NetworkError",
      message: "The upload could not be sent.",
      cause: null,
    });
  });

  it("reports a malformed successful response as a network error", async () => {
    installXhr();
    const result = uploadPhotos([]);
    lastXhr().responseText = "{";
    lastXhr().dispatchEvent(new Event("load"));

    await expect(result).rejects.toMatchObject({
      name: "NetworkError",
      message: "The upload response could not be read.",
    });
  });

  it("preserves API error fields, request id, and retry delay", async () => {
    installXhr();
    const result = uploadPhotos([]);
    const xhr = lastXhr();
    xhr.status = 429;
    xhr.statusText = "Too Many Requests";
    xhr.responseText = JSON.stringify({
      code: "rate_limited",
      message: "Try later.",
      errors: [{ field: "files", message: "Too many files." }],
    });
    xhr.headers.set("x-request-id", "request-123");
    xhr.headers.set("retry-after", "30");
    xhr.dispatchEvent(new Event("load"));

    const error = await result.catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiRequestError);
    expect(error).toMatchObject({ status: 429, code: "rate_limited", requestId: "request-123", retryAfterSeconds: 30 });
    expect((error as ApiRequestError).fieldError("files")).toBe("Too many files.");
  });

  it("uses the unauthorized error type for an upload 401", async () => {
    installXhr();
    const result = uploadPhotos([]);
    const xhr = lastXhr();
    xhr.status = 401;
    xhr.responseText = "not json";
    xhr.dispatchEvent(new Event("load"));

    await expect(result).rejects.toBeInstanceOf(UnauthorizedError);
  });

  it("falls back when an upload error body is empty or has invalid field types", async () => {
    installXhr();
    const emptyResult = uploadPhotos([]);
    const emptyXhr = lastXhr();
    emptyXhr.status = 400;
    emptyXhr.statusText = "Bad Request";
    emptyXhr.responseText = "";
    emptyXhr.dispatchEvent(new Event("load"));
    const emptyError = await emptyResult.catch((caught: unknown) => caught);
    expect(emptyError).toMatchObject({ code: "unknown", message: "Bad Request" });

    const invalidResult = uploadPhotos([]);
    const invalidXhr = lastXhr();
    invalidXhr.status = 400;
    invalidXhr.responseText = JSON.stringify({ code: 9, message: null, errors: "invalid" });
    invalidXhr.dispatchEvent(new Event("load"));
    const invalidError = await invalidResult.catch((caught: unknown) => caught);
    expect(invalidError).toMatchObject({ code: "unknown", message: "The upload failed." });
    expect((invalidError as ApiRequestError).fieldErrors).toEqual([]);

    const nullResult = uploadPhotos([]);
    const nullXhr = lastXhr();
    nullXhr.status = 400;
    nullXhr.responseText = "null";
    nullXhr.dispatchEvent(new Event("load"));
    const nullError = await nullResult.catch((caught: unknown) => caught);
    expect(nullError).toMatchObject({ code: "unknown", message: "The upload failed." });
  });
});
