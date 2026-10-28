import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { Moderation } from "../Moderation";
import { ADA, authState, renderScreen } from "./support";
const report = {
  id: "r1",
  target: "voyage:v1",
  reason: "Harassment",
  details: "Please review the content.",
  status: "open",
  created_at: ADA.created_at,
  reporter_id: "u1",
  target_owner_id: "u2",
  voyage_id: "v1",
  moderator_id: null,
  moderator_note: null,
  resolved_at: null,
};
afterEach(() => vi.unstubAllGlobals());
it("restricts report content to administrators", () => {
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  renderScreen(<Moderation />, { auth: authState(ADA) });
  expect(screen.getByText("Site administrator access is required.")).toBeVisible();
  expect(fetch).not.toHaveBeenCalled();
});
it("shows the private report and requires a review note before holding content", async () => {
  const fetch = vi.fn((_: string, init?: RequestInit) =>
    Promise.resolve(
      init?.method === "POST"
        ? new Response(null, { status: 204 })
        : Response.json({ reports: [report], next_before: null }),
    ),
  );
  vi.stubGlobal("fetch", fetch);
  renderScreen(<Moderation />, { auth: authState({ ...ADA, is_site_admin: true }) });
  await userEvent.click(await screen.findByRole("button", { name: "Review Harassment" }));
  expect(screen.getByText(report.details)).toBeVisible();
  expect(screen.getByRole("button", { name: "Hold content" })).toBeDisabled();
  await userEvent.type(screen.getByRole("textbox", { name: "Review note" }), "Under review");
  await userEvent.click(screen.getByRole("button", { name: "Hold content" }));
  await waitFor(() =>
    expect(fetch).toHaveBeenCalledWith(
      "/api/admin/reports/r1/actions",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ action: "hold", note: "Under review" }),
      }),
    ),
  );
  expect(await screen.findByText("Review saved.")).toBeVisible();
});
