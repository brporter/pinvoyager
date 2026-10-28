import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { PublicationPanel } from "../PublicationPanel";
import { renderScreen } from "../../screens/__tests__/support";
afterEach(() => vi.unstubAllGlobals());
it("shows server-armed consent and cancels it without silently unpublishing", async () => {
  const fetch = vi.fn((_: string, init?: RequestInit) =>
    Promise.resolve(
      Response.json(
        init?.method === "DELETE"
          ? { id: "i1", status: "published", published: true }
          : {
              intents: [
                {
                  id: "i1",
                  status: "processing",
                  waiting_count: 0,
                  processing_count: 2,
                  expires_at: "2026-10-30T12:00:00Z",
                  published: false,
                  revision: "17",
                  reason: null,
                },
              ],
            },
      ),
    ),
  );
  vi.stubGlobal("fetch", fetch);
  renderScreen(<PublicationPanel voyageId="v1" />);
  expect(await screen.findByText(/2 photos processing/)).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Cancel scheduled publication" }));
  await waitFor(() =>
    expect(fetch).toHaveBeenCalledWith(
      "/api/voyages/v1/publication-intents/i1",
      expect.objectContaining({ method: "DELETE" }),
    ),
  );
  expect(await screen.findByText(/already published/)).toBeVisible();
});
