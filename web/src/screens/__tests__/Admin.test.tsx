import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { Admin } from "../Admin";
import { ADA, authState, renderScreen } from "./support";

const admin = { ...ADA, is_site_admin: true };
const account = {
  id: "other",
  display_name: "sailor",
  created_at: ADA.created_at,
  is_site_admin: false,
  suspended: false,
  photo_count: 4,
  voyage_count: 2,
  storage_bytes: 4096,
};
const voyages = [
  { id: "v1", title: "Ocean", description: "Across the sea", published: true },
  { id: "v2", title: "Draft", description: null, published: false },
];
const detail = { user: account, voyages, next_before: null };
function mockFetch() {
  const fetch = vi.fn((path: string, init?: RequestInit) =>
    Promise.resolve(
      init?.method === "POST"
        ? new Response(null, { status: 204 })
        : Response.json(path.includes("/other") ? detail : { users: [account], next_before: null }),
    ),
  );
  vi.stubGlobal("fetch", fetch);
  return fetch;
}
function showDetail(user = account) {
  const fetch = mockFetch();
  fetch.mockImplementation((_: string, init?: RequestInit) =>
    Promise.resolve(
      init?.method === "POST"
        ? new Response(null, { status: 204 })
        : Response.json({ ...detail, user }),
    ),
  );
  renderScreen(<Admin />, {
    auth: authState(admin),
    path: `/admin/users/${user.id}`,
    route: "/admin/users/:id",
  });
  return fetch;
}
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("denies ordinary accounts without fetching administration data", () => {
  const fetch = mockFetch();
  renderScreen(<Admin />, { auth: authState(ADA) });
  expect(screen.getByText("Site administrator access is required.")).toBeInTheDocument();
  expect(fetch).not.toHaveBeenCalled();
});
it("lists accounts with counts and storage and follows the next page", async () => {
  const fetch = mockFetch();
  fetch.mockResolvedValueOnce(Response.json({ users: [account], next_before: "cursor" }));
  renderScreen(<Admin />, { auth: authState(admin) });
  expect(screen.getByRole("status")).toHaveTextContent("Loading");
  expect(await screen.findByRole("link", { name: "sailor" })).toHaveAttribute(
    "href",
    "/admin/users/other",
  );
  expect(screen.getByText("4.0 KiB")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next page" }));
  await waitFor(() =>
    expect(fetch).toHaveBeenCalledWith("/api/admin/users?before=cursor", expect.anything()),
  );
  await userEvent.click(screen.getByRole("button", { name: "Previous page" }));
  expect(await screen.findByRole("link", { name: "sailor" })).toBeInTheDocument();
});
it.each([
  ["Suspend account", "suspend"],
  ["Delete account and all data", "delete"],
  ["Promote to administrator", "promote"],
  ["Unpublish all voyages", "unpublish_all"],
  ["Unpublish Ocean", "unpublish_voyage"],
])("requires confirmation for %s and sends the selected action", async (label, action) => {
  const fetch = showDetail();
  expect(await screen.findByText("Across the sea")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: label }));
  let dialog = screen.getByRole("dialog");
  expect(fetch.mock.calls.filter(([, init]) => init?.method === "POST")).toHaveLength(0);
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: label }));
  dialog = screen.getByRole("dialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "Confirm" }));
  await waitFor(() =>
    expect(fetch).toHaveBeenCalledWith(
      "/api/admin/users/other/actions",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify(
          action === "unpublish_voyage" ? { action, voyage_id: "v1" } : { action },
        ),
      }),
    ),
  );
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});
it("offers confirmed account re-enabling and omits promotion for admins", async () => {
  const fetch = showDetail({ ...account, suspended: true, is_site_admin: true });
  await userEvent.click(await screen.findByRole("button", { name: "Enable account" }));
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Confirm" }),
  );
  await waitFor(() =>
    expect(fetch).toHaveBeenCalledWith(
      "/api/admin/users/other/actions",
      expect.objectContaining({ body: JSON.stringify({ action: "enable" }) }),
    ),
  );
  expect(
    screen.queryByRole("button", { name: "Promote to administrator" }),
  ).not.toBeInTheDocument();
});
it("disables self suspension and deletion", async () => {
  showDetail({ ...account, id: ADA.id, is_site_admin: true });
  expect(await screen.findByRole("button", { name: "Suspend account" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Delete account and all data" })).toBeDisabled();
});
it("keeps a failed action in the dialog for retry", async () => {
  const fetch = showDetail();
  await userEvent.click(await screen.findByRole("button", { name: "Suspend account" }));
  fetch.mockResolvedValueOnce(
    Response.json({ message: "Action refused", code: "conflict" }, { status: 409 }),
  );
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Confirm" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("Action refused");
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Confirm" }),
  );
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});
it("retries failed account reads", async () => {
  const fetch = mockFetch();
  fetch.mockRejectedValueOnce(new Error("offline"));
  renderScreen(<Admin />, { auth: authState(admin) });
  await userEvent.click(await screen.findByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("link", { name: "sailor" })).toBeInTheDocument();
});

it("shows empty accounts and denies anonymous access", async () => {
  const fetch = mockFetch();
  fetch.mockResolvedValue(Response.json({ users: [], next_before: null }));
  const rendered = renderScreen(<Admin />, { auth: authState(admin) });
  expect(await screen.findByText("No users found.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
  rendered.unmount();
  renderScreen(<Admin />);
  expect(screen.getByText("Site administrator access is required.")).toBeInTheDocument();
});

it("paginates voyage metadata and shows accounts without display names or voyages", async () => {
  const fetch = mockFetch();
  fetch.mockResolvedValueOnce(
    Response.json({
      ...detail,
      user: { ...account, display_name: null, storage_bytes: 3 * 1024 ** 3 },
      next_before: "v1",
    }),
  );
  fetch.mockResolvedValueOnce(
    Response.json({
      ...detail,
      user: { ...account, display_name: null, storage_bytes: 0 },
      voyages: [],
    }),
  );
  renderScreen(<Admin />, {
    auth: authState(admin),
    path: "/admin/users/other",
    route: "/admin/users/:id",
  });
  expect(await screen.findByRole("heading", { name: "other" })).toBeInTheDocument();
  expect(screen.getByText(/3.0 GiB stored/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Suspend account" }));
  expect(within(screen.getByRole("dialog")).getByText("Account: other")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await userEvent.click(screen.getByRole("button", { name: "Next page" }));
  expect(await screen.findByText("No voyages found.")).toBeInTheDocument();
  expect(fetch).toHaveBeenCalledWith("/api/admin/users/other?before=v1", expect.anything());
  expect(screen.getByText(/0 B stored/)).toBeInTheDocument();
});

it("shows suspension, administrator role, and ID fallback in the account list", async () => {
  const fetch = mockFetch();
  fetch.mockResolvedValue(
    Response.json({
      users: [{ ...account, display_name: null, suspended: true, is_site_admin: true }],
      next_before: null,
    }),
  );
  renderScreen(<Admin />, { auth: authState(admin) });
  expect(await screen.findByRole("link", { name: "other" })).toBeInTheDocument();
  expect(screen.getByRole("cell", { name: "Suspended" })).toBeInTheDocument();
  expect(screen.getByRole("cell", { name: "Site administrator" })).toBeInTheDocument();
});

it("prevents dismissal and duplicate submission while confirmation is pending", async () => {
  const { fireEvent } = await import("@testing-library/react");
  const fetch = showDetail();
  await userEvent.click(await screen.findByRole("button", { name: "Suspend account" }));
  const dialog = screen.getByRole("dialog");
  expect(fireEvent(dialog, new Event("cancel", { cancelable: true, bubbles: true }))).toBe(true);
  let finish!: (response: Response) => void;
  fetch.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await userEvent.click(within(dialog).getByRole("button", { name: "Confirm" }));
  expect(within(dialog).getByRole("button", { name: "Confirm" })).toBeDisabled();
  expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
  expect(fireEvent(dialog, new Event("cancel", { cancelable: true, bubbles: true }))).toBe(false);
  fireEvent.pointerDown(dialog, { button: 0, clientX: -10, clientY: -10 });
  fireEvent.click(dialog, { button: 0, clientX: -10, clientY: -10 });
  expect(dialog).toBeInTheDocument();
  finish(new Response(null, { status: 204 }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(screen.getByRole("status")).toHaveTextContent("Action completed.");
});
