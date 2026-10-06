import { StrictMode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  createRoot: vi.fn(),
  rootRender: vi.fn(),
  clientOptions: undefined as
    | {
        defaultOptions: {
          queries: {
            retry: (failureCount: number, error: unknown) => boolean;
          };
        };
      }
    | undefined,
}));

vi.mock("react-dom/client", () => ({
  createRoot: mocks.createRoot,
}));

vi.mock("@tanstack/react-query", () => ({
  QueryClient: class {
    constructor(options: typeof mocks.clientOptions) {
      mocks.clientOptions = options;
    }
  },
  QueryClientProvider: () => null,
}));

vi.mock("../App", () => ({ App: () => null }));

describe("application startup", () => {
  afterEach(() => {
    vi.resetModules();
    mocks.createRoot.mockReset();
    mocks.rootRender.mockReset();
    mocks.createRoot.mockReturnValue({ render: mocks.rootRender });
    mocks.clientOptions = undefined;
  });

  it("mounts the app under StrictMode at the root element", async () => {
    document.body.innerHTML = '<div id="root"></div>';
    mocks.createRoot.mockReturnValue({ render: mocks.rootRender });

    await import("../main");

    expect(mocks.createRoot).toHaveBeenCalledOnce();
    expect(mocks.createRoot).toHaveBeenCalledWith(document.getElementById("root"));
    expect(mocks.rootRender).toHaveBeenCalledOnce();
    expect(mocks.rootRender.mock.calls[0]?.[0].type).toBe(StrictMode);
  });

  it("does not retry unauthorized queries and retries one other failure", async () => {
    document.body.innerHTML = '<div id="root"></div>';
    mocks.createRoot.mockReturnValue({ render: mocks.rootRender });
    await import("../main");

    const retry = mocks.clientOptions?.defaultOptions.queries.retry;
    expect(retry).toBeTypeOf("function");
    expect(retry?.(0, new Response(null, { status: 401 }))).toBe(false);
    expect(retry?.(0, new Response(null, { status: 500 }))).toBe(true);
    expect(retry?.(0, new Error("network failure"))).toBe(true);
    expect(retry?.(1, new Error("already retried"))).toBe(false);
  });
});
