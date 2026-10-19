import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { afterEach, expect, it, vi } from "vitest";
import { App } from "../../App";

// Legal documents must render even when the session API is unavailable.
afterEach(() => vi.unstubAllGlobals());

it.each([
  ["/terms", "Terms of Service"],
  ["/privacy", "Privacy Policy"],
])("serves %s without a session or profile setup", async (path, title) => {
  const fetch = vi.fn(() => Promise.reject(new Error("API unavailable")));
  vi.stubGlobal("fetch", fetch);
  render(
    <MemoryRouter initialEntries={[path]}>
      <App />
    </MemoryRouter>,
  );
  expect(await screen.findByRole("heading", { level: 1, name: title })).toBeInTheDocument();
  expect(screen.getByText(/Draft for review/)).toBeInTheDocument();
  expect(fetch).not.toHaveBeenCalled();
});

it("explains content responsibility, discretionary removal, and referrals", () => {
  render(
    <MemoryRouter initialEntries={["/terms"]}>
      <App />
    </MemoryRouter>,
  );
  const article = screen.getByRole("article");
  expect(article).toHaveTextContent(/at any time, for any reason/);
  expect(article).toHaveTextContent(/sole discretion/);
  expect(article).toHaveTextContent(/without warning/);
  expect(article).toHaveTextContent(/not responsible for user content/);
  expect(article).toHaveTextContent(/deem illegal will be referred to the appropriate authorities/);
  expect(within(article).getByRole("link", { name: /App.net/ })).toHaveAttribute(
    "href",
    "https://github.com/appdotnet/template-terms-of-service",
  );
});

it("explains stored originals, exact locations, public sharing, and deletion limits", () => {
  render(
    <MemoryRouter initialEntries={["/privacy"]}>
      <App />
    </MemoryRouter>,
  );
  const article = screen.getByRole("article");
  expect(article).toHaveTextContent(/original uploads/);
  expect(article).toHaveTextContent(/exact coordinates/);
  expect(article).toHaveTextContent(/Published voyages are public/);
  expect(article).toHaveTextContent(/backups/);
  expect(article).toHaveTextContent(/no self-service account deletion/);
});

it("navigates between legal pages from the footer", async () => {
  const user = userEvent.setup();
  render(
    <MemoryRouter initialEntries={["/terms"]}>
      <App />
    </MemoryRouter>,
  );
  await user.click(
    within(screen.getByRole("contentinfo")).getByRole("link", { name: "Privacy Policy" }),
  );
  expect(screen.getByRole("heading", { level: 1, name: "Privacy Policy" })).toBeInTheDocument();
  await user.click(
    within(screen.getByRole("contentinfo")).getByRole("link", { name: "Terms of Service" }),
  );
  expect(screen.getByRole("heading", { level: 1, name: "Terms of Service" })).toBeInTheDocument();
});
