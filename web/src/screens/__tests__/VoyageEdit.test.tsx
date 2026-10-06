/**
 * Behavior of the voyage create/edit screen.
 *
 * The two things worth protecting here are the ones that are silently wrong
 * rather than visibly broken: a validation message that lands in a banner
 * instead of beside the input it is about, and a precision control that
 * displays a value it is not actually holding.
 */

import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UseMutationResult, UseQueryResult } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiRequestError } from "../../api/client";
import type { UpdateVoyageInput } from "../../api/client";
import { useDeleteVoyage, useUpdateVoyage, useVoyage } from "../../api/hooks";
import type { Voyage, VoyagePhoto } from "../../api/types";
import { VoyageEdit } from "../VoyageEdit";

vi.mock("../../api/hooks", () => ({
  useVoyage: vi.fn(),
  useUpdateVoyage: vi.fn(),
  useDeleteVoyage: vi.fn(),
}));

/* -- fixtures ------------------------------------------------------------ */

function photo(overrides: Partial<VoyagePhoto> & { id: string }): VoyagePhoto {
  return {
    position: 1,
    captured_at: "2025-02-14T09:12:00Z",
    width: 4032,
    height: 3024,
    variant_status: "ready",
    thumbnail_url: `/api/photos/${overrides.id}/thumb`,
    display_url: `/api/photos/${overrides.id}/display`,
    full_url: `/api/photos/${overrides.id}/full`,
    coordinate: { latitude: 43.06, longitude: 141.35, precision_m: 1000 },
    placed: true,
    ...overrides,
  };
}

function voyage(overrides: Partial<Voyage> = {}): Voyage {
  return {
    id: "v1",
    owner_id: "u1",
    title: "Three weeks in Hokkaido",
    description: "Late winter, mostly on foot.",
    published: false,
    granularity: "km10",
    granularity_label: "~10 km",
    precision_m: 5000,
    created_at: "2026-02-01T10:00:00Z",
    published_at: null,
    photo_count: 2,
    placed_count: 1,
    place_count: 1,
    unplaced_count: 1,
    photos: [
      photo({ id: "p1", position: 1 }),
      photo({ id: "p2", position: 2, coordinate: null, placed: false }),
    ],
    ...overrides,
  };
}

/* -- harness ------------------------------------------------------------- */

function queryResult(partial: Partial<UseQueryResult<Voyage>>): UseQueryResult<Voyage> {
  return {
    isPending: false,
    isError: false,
    isSuccess: true,
    refetch: vi.fn(),
    ...partial,
  } as unknown as UseQueryResult<Voyage>;
}

type UpdateResult = UseMutationResult<Voyage, Error, UpdateVoyageInput>;

function mutationResult(partial: {
  mutate?: UpdateResult["mutate"];
  isPending?: boolean;
  error?: Error | null;
}): UpdateResult {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn(),
    isPending: false,
    error: null,
    ...partial,
  } as unknown as UpdateResult;
}

interface Setup {
  data?: Voyage;
  update?: Parameters<typeof mutationResult>[0];
  remove?: ReturnType<typeof vi.fn>;
  removeError?: Error | null;
}

function setup({ data = voyage(), update = {}, remove = vi.fn(), removeError = null }: Setup = {}) {
  const mutate = update.mutate ?? vi.fn();
  vi.mocked(useVoyage).mockReturnValue(queryResult({ data }));
  vi.mocked(useUpdateVoyage).mockReturnValue(mutationResult({ ...update, mutate }));
  vi.mocked(useDeleteVoyage).mockReturnValue(
    mutationResult({ mutate: remove as UpdateResult["mutate"], error: removeError }) as unknown as ReturnType<typeof useDeleteVoyage>,
  );

  const view = render(
    <MemoryRouter initialEntries={["/voyages/v1/edit"]}>
      <Routes>
        <Route path="/voyages/:id/edit" element={<VoyageEdit />} />
        <Route path="*" element={<span>elsewhere</span>} />
      </Routes>
    </MemoryRouter>,
  );

  return { mutate, view };
}

function renderAt(path: string, route = "/voyages/:id/edit") {
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path={route} element={<VoyageEdit />} />
        <Route path="*" element={<span>elsewhere</span>} />
      </Routes>
    </MemoryRouter>,
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

/* -- tests --------------------------------------------------------------- */

describe("VoyageEdit — validation", () => {
  it("puts a field error beside the input it names, not in a banner", () => {
    setup({
      update: {
        error: new ApiRequestError({
          status: 422,
          code: "validation_failed",
          message: "A title is required before publishing.",
          requestId: null,
          fieldErrors: [{ field: "title", message: "A title is required before publishing." }],
        }),
      },
    });

    const title = screen.getByLabelText(/Title/);
    expect(title).toHaveAttribute("aria-invalid", "true");
    // Described by the message, so it is announced on focus rather than left
    // to be discovered by trial.
    expect(title).toHaveAccessibleDescription(/A title is required before publishing/);

    // And it lives inside the field, not above the form.
    const field = title.closest(".pv-field");
    expect(field).not.toBeNull();
    expect(
      within(field as HTMLElement).getByText(/A title is required before publishing/),
    ).toBeInTheDocument();
  });

  it("leaves an unattributed failure as a message of its own", () => {
    setup({
      update: {
        error: new ApiRequestError({
          status: 500,
          code: "internal",
          message: "Something went wrong.",
          requestId: "req-9",
        }),
      },
    });

    expect(screen.getByRole("alert")).toHaveTextContent("Something went wrong.");
    expect(screen.getByLabelText(/Title/)).not.toHaveAttribute("aria-invalid");
  });
});

describe("VoyageEdit — precision", () => {
  it("shows the precision the voyage is currently at", () => {
    setup();

    const steps = screen.getByRole("group", { name: "Location precision" });
    expect(within(steps).getByRole("button", { name: "~10 km" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(within(steps).getByRole("button", { name: "Exact" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("reports a change, and saves the value that was chosen", async () => {
    const { mutate } = setup();

    await userEvent.click(screen.getByRole("button", { name: "~1 km" }));

    const steps = screen.getByRole("group", { name: "Location precision" });
    expect(within(steps).getByRole("button", { name: "~1 km" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(within(steps).getByRole("button", { name: "~10 km" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    await userEvent.click(screen.getByRole("button", { name: "Save draft" }));

    expect(mutate).toHaveBeenCalledWith(
      expect.objectContaining({ granularity: "km1" }),
      expect.anything(),
    );
  });

  it("states the stripping guarantee where the choice is made", () => {
    setup();

    expect(
      screen.getByText(/Location data is removed from the files themselves/),
    ).toBeInTheDocument();
  });

  it("puts the precision control immediately before the publish button", () => {
    setup();

    const steps = screen.getByRole("group", { name: "Location precision" });
    const publish = screen.getByRole("button", { name: "Publish voyage" });

    // Adjacency is the argument: the last thing seen before making a voyage
    // public is the precision it will be published at.
    expect(
      steps.compareDocumentPosition(publish) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("is inert when there is nothing to coarsen", () => {
    setup({
      data: voyage({ placed_count: 0, photos: [photo({ id: "p2", coordinate: null, placed: false })] }),
    });

    expect(screen.getByRole("button", { name: "~10 km" })).toBeDisabled();
    expect(screen.getByText(/nothing to coarsen yet/)).toBeInTheDocument();
  });
});

describe("VoyageEdit — publishing", () => {
  it("publishes with the metadata on screen", async () => {
    const { mutate } = setup();

    await userEvent.clear(screen.getByLabelText(/Title/));
    await userEvent.type(screen.getByLabelText(/Title/), "Hokkaido");
    await userEvent.click(screen.getByRole("button", { name: "Publish voyage" }));

    expect(mutate).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Hokkaido", published: true }),
      expect.anything(),
    );
  });

  it("discards changes and returns to the voyage", async () => {
    setup();
    await userEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(screen.getByText("elsewhere")).toBeInTheDocument();
  });

  it("clears the description rather than leaving it alone when it is emptied", async () => {
    const { mutate } = setup();

    await userEvent.clear(screen.getByLabelText("Description"));
    await userEvent.click(screen.getByRole("button", { name: "Save draft" }));

    expect(mutate).toHaveBeenCalledWith(
      expect.objectContaining({ description: null }),
      expect.anything(),
    );
  });

  it("starts with an empty description when the voyage has none", () => {
    setup({ data: voyage({ description: null }) });
    expect(screen.getByLabelText("Description")).toHaveValue("");
  });

  it("unpublishes a public voyage", async () => {
    const { mutate } = setup({ data: voyage({ published: true }) });
    await userEvent.click(screen.getByRole("button", { name: "Unpublish voyage" }));
    expect(mutate).toHaveBeenCalledWith(expect.objectContaining({ published: false }), expect.anything());
  });

  it("offers to unpublish an already public voyage", () => {
    setup({ data: voyage({ published: true, published_at: "2026-02-02T10:00:00Z" }) });

    expect(screen.getByRole("button", { name: "Unpublish voyage" })).toBeInTheDocument();
    expect(screen.getByText("Public")).toBeInTheDocument();
  });
});

describe("VoyageEdit — photographs", () => {
  it("shows unplaced photographs in the grid rather than filtering them out", () => {
    setup();

    const grid = screen.getByRole("group", { name: "Photos in this voyage" });
    const tiles = grid.querySelectorAll(".pv-photo");
    expect(tiles).toHaveLength(2);
    expect(grid.querySelectorAll('[data-unplaced="true"]')).toHaveLength(1);
    expect(screen.getByText(/1 photo with no location data/)).toBeInTheDocument();
  });

  it("uses the plural unplaced-photo explanation", () => {
    setup({
      data: voyage({
        photos: [
          photo({ id: "p1", position: 1 }),
          photo({ id: "p2", position: 2, coordinate: null, placed: false }),
          photo({ id: "p3", position: 3, coordinate: null, placed: false }),
        ],
        photo_count: 3,
        unplaced_count: 2,
      }),
    });
    expect(screen.getByText(/2 photos with no location data/)).toBeInTheDocument();
  });

  it("uses the wide edit layout on desktop", () => {
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query.includes("min-width: 768px"),
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
    }));
    setup();
    expect(document.querySelector('div[style*="grid-template-columns"]'))
      .toHaveStyle({ gridTemplateColumns: "1fr 360px" });
  });
});

describe("VoyageEdit — states", () => {
  it("shows the unavailable state when no voyage id is present", () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({}));
    renderAt("/edit", "/edit");

    expect(screen.getByText("This voyage is not here")).toBeInTheDocument();
  });

  it("shows the unavailable state for a missing voyage", () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ isSuccess: false, isError: true, error: new ApiRequestError({
        status: 404, code: "not_found", message: "Missing", requestId: null,
      }) }),
    );
    renderAt("/voyages/v1/edit");

    expect(screen.getByText("This voyage is not here")).toBeInTheDocument();
  });

  it("renders a retry message without a request reference for a regular error", () => {
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({ isSuccess: false, isError: true, error: new Error("offline") }),
    );
    renderAt("/voyages/v1/edit");

    expect(screen.getByText(/could not be loaded/)).toBeInTheDocument();
    expect(screen.queryByText(/Quote reference/)).not.toBeInTheDocument();
  });

  it("says it is loading rather than rendering an empty form", () => {
    vi.mocked(useVoyage).mockReturnValue(queryResult({ isPending: true, isSuccess: false }));
    vi.mocked(useUpdateVoyage).mockReturnValue(mutationResult({}));
    vi.mocked(useDeleteVoyage).mockReturnValue(
      mutationResult({}) as unknown as ReturnType<typeof useDeleteVoyage>,
    );

    render(
      <MemoryRouter initialEntries={["/voyages/v1/edit"]}>
        <Routes>
          <Route path="/voyages/:id/edit" element={<VoyageEdit />} />
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.getByText("Loading this voyage…")).toBeInTheDocument();
  });

  it("offers a retry when the voyage cannot be read", async () => {
    const refetch = vi.fn();
    vi.mocked(useVoyage).mockReturnValue(
      queryResult({
        isPending: false,
        isSuccess: false,
        isError: true,
        refetch,
        error: new ApiRequestError({
          status: 503,
          code: "service_unavailable",
          message: "Try later.",
          requestId: "req-3",
        }),
      }),
    );
    vi.mocked(useUpdateVoyage).mockReturnValue(mutationResult({}));
    vi.mocked(useDeleteVoyage).mockReturnValue(
      mutationResult({}) as unknown as ReturnType<typeof useDeleteVoyage>,
    );

    render(
      <MemoryRouter initialEntries={["/voyages/v1/edit"]}>
        <Routes>
          <Route path="/voyages/:id/edit" element={<VoyageEdit />} />
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.getByText(/req-3/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(refetch).toHaveBeenCalled();
  });

  it("shows an empty-photo state when the voyage has no photos", () => {
    setup({ data: voyage({ photo_count: 0, photos: [] }) });

    expect(screen.getByText("No photographs yet")).toBeInTheDocument();
  });

  it("shows successful save feedback and allows it to be dismissed", async () => {
    const { mutate } = setup({ update: { mutate: vi.fn((_input, options) => options?.onSuccess?.(voyage(), undefined, undefined)) } });
    await userEvent.click(screen.getByRole("button", { name: "Save draft" }));

    expect(mutate).toHaveBeenCalled();
    expect(screen.getByText("Your changes are saved.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Your changes are saved.")).not.toBeInTheDocument();
  });

  it("shows published and unpublished feedback after successful changes", async () => {
    const mutate = vi.fn((_input, options) => options?.onSuccess?.(voyage(), undefined, undefined));
    const { view } = setup({ update: { mutate } });

    await userEvent.click(screen.getByRole("button", { name: "Publish voyage" }));
    expect(screen.getByText(/Locations are shown at ~10 km/)).toBeInTheDocument();
    view.unmount();

    const unpublish = vi.fn((_input, options) => options?.onSuccess?.(voyage(), undefined, undefined));
    setup({ data: voyage({ published: true }), update: { mutate: unpublish } });
    await userEvent.click(screen.getByRole("button", { name: "Unpublish voyage" }));
    expect(screen.getByText("Your changes are saved.")).toBeInTheDocument();
  });

  it("confirms and cancels voyage deletion", async () => {
    setup();
    await userEvent.click(screen.getByRole("button", { name: "Delete voyage" }));
    expect(screen.getByRole("button", { name: "Delete permanently" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Keep it" }));
    expect(screen.queryByRole("button", { name: "Delete permanently" })).not.toBeInTheDocument();
  });

  it("shows a deletion error and a fallback when its message is empty", () => {
    setup({ removeError: new Error("") });
    expect(screen.getByRole("alert")).toHaveTextContent("That did not delete");
    expect(screen.getByRole("alert")).toHaveTextContent("Something went wrong at our end.");
  });

  it("deletes the voyage and returns to discovery after success", async () => {
    const remove = vi.fn((_id, options) => options?.onSuccess?.());
    setup({ remove });
    await userEvent.click(screen.getByRole("button", { name: "Delete voyage" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete permanently" }));
    expect(remove).toHaveBeenCalledWith("v1", expect.anything());
    expect(await screen.findByText("elsewhere")).toBeInTheDocument();
  });

  it("submits Enter as a draft save", async () => {
    const { mutate } = setup();
    fireEvent.submit(screen.getByLabelText(/Title/).closest("form") as HTMLFormElement);
    expect(mutate).toHaveBeenCalledWith(expect.objectContaining({
      title: "Three weeks in Hokkaido",
      description: "Late winter, mostly on foot.",
      granularity: "km10",
    }), expect.anything());
  });
});
