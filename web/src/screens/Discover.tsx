/**
 * Discover — implements `design/screens/discover.html`.
 *
 * The public landing page and the browse surface. Every voyage on PinVoyager is
 * public, so a signed-out visitor is a first-class user here rather than
 * someone being held at a gate: the hero, the grid and the pagination all work
 * with no session at all.
 *
 * Three decisions come straight from the design's annotations:
 *
 *  1. **Each card carries the voyage's precision label.** A browser scanning
 *     the grid can see that one creator publishes exact coordinates and another
 *     publishes to ~50 km, which sets expectations before the map even loads.
 *     `granularity_label` is rendered rather than derived, so the three places
 *     it appears cannot disagree.
 *  2. **Three columns on desktop, one on mobile.** Two columns at 390px would
 *     put each cover at roughly 175px wide, which is too small for a photograph
 *     to do its job as the reason to click. The breakpoint is evaluated in
 *     JavaScript by `useIsDesktop` because `design/components.css` carries no
 *     layout media queries — see the note on styling below.
 *  3. **The hero states the mechanism, not a slogan.** "PinVoyager reads the
 *     location your camera recorded" tells a new visitor what the product does
 *     *and* where the data comes from, which is what they need to understand
 *     before uploading anything.
 *
 * ## Styling
 *
 * No CSS is added by this file. `design/components.css` defines the card, the
 * badge, the empty state and the skeleton; what it does not define is page
 * furniture — the hero band, the three-column grid, the section padding. The
 * design file draws those with a per-screen `<style>` block, so the same
 * declarations appear here as inline styles referring to the same tokens. A
 * token still owns every value; only the arrangement is local.
 *
 * ## What the API cannot yet supply
 *
 * `VoyageSummary` carries no cover photograph, no photo count and no creator
 * name — only `owner_id`. The design's card meta ("Bryan Porter · 48 photos ·
 * ~1 km") therefore cannot be rendered in full: the cover frame is drawn empty
 * so the grid keeps the proportions the design specifies, and the meta line
 * shows the precision label and the publication date, which are the fields that
 * exist. Filling the rest needs `VoyageSummary` to grow a thumbnail and a count,
 * not a change here.
 */

import type { CSSProperties, ReactNode } from "react";
import { Link } from "react-router";

import { isApiError } from "../api/client";
import { useVoyages } from "../api/hooks";
import type { VoyageList, VoyageSummary } from "../api/types";
import { signInPath } from "../auth/redirect";
import { useAuth } from "../auth/useAuth";
import { Badge, Button, EmptyState, Photo, SearchIcon, Skeleton } from "../components";
import { LazyDiscoverMap } from "../map";
import { useIsDesktop } from "../shell/useIsDesktop";
import { ErrorScreen } from "./NotFound";

// ---------------------------------------------------------------------------
// Shared with Profile
//
// `Profile` lists the same cards in the same grid, so the pieces below are
// exported rather than written twice. They live here because Discover is where
// the design specifies them.
// ---------------------------------------------------------------------------

/**
 * Zeroes the two radius tokens *inside a card's cover frame only*.
 *
 * `.pv-voyage-card` clips its children to a 12px radius, and `.pv-photo`
 * rounds itself by the same amount — so the cover's lower corners would show
 * two notches of card background. Every card in `design/screens/discover.html`
 * fixes this with `style="border-radius:0"` on the frame. `Photo` takes a
 * `className` but no `style`, and inventing a `.pv-photo--flush` class would be
 * writing CSS, so the token the rule reads is overridden for that subtree
 * instead. The badge inside is unaffected: it rounds by `--pv-radius-full`.
 */
const FLUSH_COVER = {
  "--pv-radius-lg": "0px",
  "--pv-radius-md": "0px",
} as unknown as CSSProperties;

/** Every voyage across every page fetched so far. */
export function voyagesOf(
  pages: { pages: VoyageList[] } | undefined,
): VoyageSummary[] {
  return pages?.pages.flatMap((page) => page.voyages) ?? [];
}

/** "August 2026". The month is enough — a voyage is not a news item, and a
 *  precise timestamp invites reading a publication time as a travel date. */
export function monthYear(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

export interface VoyageGridProps {
  desktop: boolean;
  /** Announced once for the whole region rather than per skeleton — a skeleton
   *  is `aria-hidden`, so this is the only thing that says "loading". */
  busy?: boolean;
  "aria-labelledby"?: string;
  "aria-label"?: string;
  children: ReactNode;
}

/** Three columns at `md` and above, one below. `minmax(0, 1fr)` rather than
 *  `1fr`: a grid track's default `min-width: auto` lets a long unbroken title
 *  push the column wider than its share, which is one of the two ways a page
 *  acquires a horizontal scrollbar at 390px. */
export function VoyageGrid({ desktop, busy, children, ...naming }: VoyageGridProps) {
  return (
    <ul
      {...naming}
      aria-busy={busy === true ? true : undefined}
      style={{
        listStyle: "none",
        margin: 0,
        padding: 0,
        display: "grid",
        gridTemplateColumns: desktop ? "repeat(3, minmax(0, 1fr))" : "minmax(0, 1fr)",
        gap: desktop ? "var(--pv-space-5)" : "var(--pv-space-4)",
      }}
    >
      {children}
    </ul>
  );
}

export interface VoyageCardProps {
  voyage: VoyageSummary;
  desktop: boolean;
  /** Drawn over the cover, as `design/screens/profile.html` draws it. Only the
   *  owner is ever passed one: a draft is invisible to everybody else, and the
   *  server answers 404 rather than 403 so that its existence does not leak. */
  draftBadge?: boolean;
}

export function VoyageCard({ voyage, desktop, draftBadge = false }: VoyageCardProps) {
  const draft = draftBadge && !voyage.published;

  return (
    <li>
      <Link className="pv-voyage-card" to={`/voyages/${voyage.id}`}>
        <div style={FLUSH_COVER}>
          {/* No cover photograph exists on `VoyageSummary`; the frame is drawn
              anyway so the grid keeps its proportions and so the skeleton that
              precedes it is the same shape as what arrives. */}
          <Photo badge={draft ? <Badge tone="draft">Draft</Badge> : undefined} />
        </div>

        <div className="pv-voyage-card-body">
          <h3
            className="pv-voyage-card-title"
            style={desktop ? undefined : { fontSize: "var(--pv-text-lg)" }}
          >
            {voyage.title}
          </h3>
          <div className="pv-voyage-card-meta">
            {/* The design's annotation 1. `feedback.html` specifies the accent
                badge for a precision label; discover.html draws it as plain
                text in the meta line. The badge is used because it is the
                component that exists and it survives being scanned. */}
            <Badge tone="accent">{voyage.granularity_label}</Badge>
            <span>
              {draft
                ? "Not published"
                : monthYear(voyage.published_at ?? voyage.created_at)}
            </span>
          </div>
        </div>
      </Link>
    </li>
  );
}

/**
 * A card-shaped skeleton: cover frame, title line, meta line.
 *
 * The point is that nothing moves when the real cards arrive, so the cover is
 * the same 4:3 frame and the two text lines sit at the same heights as the
 * title and meta they stand in for.
 */
export function VoyageCardSkeleton() {
  return (
    <li className="pv-voyage-card" aria-hidden="true">
      <div style={FLUSH_COVER}>
        <Photo loading />
      </div>
      <div className="pv-voyage-card-body">
        <div className="pv-stack-2">
          <Skeleton height={22} width="70%" />
          <Skeleton height={14} width="45%" />
        </div>
      </div>
    </li>
  );
}

/** Enough skeleton cards to fill the first row and suggest the second. */
export function VoyageGridSkeleton({ desktop, count }: { desktop: boolean; count?: number }) {
  const total = count ?? (desktop ? 6 : 2);
  return (
    <VoyageGrid desktop={desktop} busy aria-label="Loading voyages">
      {Array.from({ length: total }, (_unused, index) => (
        <VoyageCardSkeleton key={index} />
      ))}
    </VoyageGrid>
  );
}

/**
 * The cursor-paginated "Load more" control.
 *
 * The API hands back an opaque `next_before` and no page number, so there is
 * nothing to number — a voyage published between two requests shifts every
 * offset, which is why the server is cursor-paginated in the first place.
 * Rendered as nothing at the end of the list rather than as a disabled button:
 * a permanently dead control is worse than no control.
 */
export function LoadMore({
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
}: {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
}) {
  if (!hasNextPage) return null;

  return (
    <div style={{ display: "flex", justifyContent: "center", marginTop: "var(--pv-space-6)" }}>
      <Button
        variant="secondary"
        size="lg"
        loading={isFetchingNextPage}
        onClick={onLoadMore}
      >
        Load more voyages
      </Button>
    </div>
  );
}

/** The `x-request-id` behind a failure, for the error screen to quote. Null
 *  when the failure never reached the service. */
export function referenceOf(error: unknown): string | null {
  return isApiError(error) ? error.requestId : null;
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

const GRID_HEADING_ID = "pv-discover-recent";
const MAP_HEADING_ID = "pv-discover-map";

export function Discover() {
  const desktop = useIsDesktop();
  const { user } = useAuth();
  const feed = useVoyages();

  const voyages = voyagesOf(feed.data);

  // Signed out, "Create a voyage" routes through sign-in carrying its
  // destination rather than disappearing — the design's annotation 4 makes the
  // same point about the tab bar.
  const createHref = user === null ? signInPath("/upload") : "/upload";

  return (
    <div>
      <section
        style={{
          padding: desktop
            ? "var(--pv-space-10) var(--pv-space-6)"
            : "var(--pv-space-8) var(--pv-space-4)",
          textAlign: "center",
          background: "var(--pv-bg-subtle)",
          borderBottom: "1px solid var(--pv-border)",
        }}
      >
        {desktop ? (
          <>
            <h1 className="pv-h1" style={{ maxWidth: "16ch", margin: "0 auto" }}>
              Photographs, placed where they were taken.
            </h1>
            <p
              className="pv-muted"
              style={{ maxWidth: "52ch", margin: "var(--pv-space-4) auto 0" }}
            >
              Upload your pictures and PinVoyager reads the location your camera recorded,
              putting each one on a map. You choose how precise that is.
            </p>
            <div
              className="pv-inline-3"
              style={{ justifyContent: "center", marginTop: "var(--pv-space-6)" }}
            >
              <Link className="pv-btn pv-btn--primary pv-btn--lg" to={createHref}>
                Create a voyage
              </Link>
              {/* A same-page jump, so a plain anchor rather than a Link: this
                  is a scroll, not a navigation, and the router has nothing to
                  do with it. */}
              <a className="pv-btn pv-btn--ghost pv-btn--lg" href={`#${GRID_HEADING_ID}`}>
                Browse voyages
              </a>
            </div>
          </>
        ) : (
          <>
            <h1 className="pv-h2">Photographs, placed where they were taken.</h1>
            <p className="pv-sm pv-muted" style={{ marginTop: "var(--pv-space-3)" }}>
              PinVoyager reads the location your camera recorded. You choose how precise it is.
            </p>
            <Link
              className="pv-btn pv-btn--primary pv-btn--lg"
              to={createHref}
              style={{ width: "100%", marginTop: "var(--pv-space-5)" }}
            >
              Create a voyage
            </Link>
          </>
        )}
      </section>

      {/* The world map, between the hero and the grid.

          `design/screens/discover.html` draws no map at all; the phase 7 goal
          asks for one on this screen, so this is a placement decision the
          design file has not made and should. The height is stated in `dvh` so
          the map is a band rather than a full screen — the grid below it is
          still the primary object on this page, and a map that filled the
          viewport would read as the whole screen. */}
      <section
        aria-labelledby={MAP_HEADING_ID}
        style={{ padding: desktop ? "var(--pv-space-6)" : "var(--pv-space-4)" }}
      >
        <h2 className="pv-h3" id={MAP_HEADING_ID} style={{ marginBottom: "var(--pv-space-4)" }}>
          Everywhere people have been
        </h2>
        <LazyDiscoverMap
          label="A map of every published photograph"
          style={{
            height: desktop ? "48dvh" : "40dvh",
            minHeight: 280,
            borderRadius: "var(--pv-radius-lg)",
          }}
        />
      </section>

      <section
        aria-labelledby={GRID_HEADING_ID}
        style={{ padding: desktop ? "var(--pv-space-6)" : "var(--pv-space-4)" }}
      >
        <h2
          className="pv-h3"
          id={GRID_HEADING_ID}
          // The jump target from "Browse voyages". Focusable so the keyboard
          // caret follows the scroll instead of staying in the hero.
          tabIndex={-1}
          style={{ marginBottom: "var(--pv-space-5)", outline: "none" }}
        >
          Recent voyages
        </h2>

        {feed.isPending ? (
          <VoyageGridSkeleton desktop={desktop} />
        ) : feed.isError ? (
          <ErrorScreen
            title="Voyages could not be loaded"
            reference={referenceOf(feed.error)}
            onRetry={() => {
              void feed.refetch();
            }}
          />
        ) : voyages.length === 0 ? (
          <EmptyState
            icon={<SearchIcon size={40} strokeWidth={1.5} />}
            title="No public voyages yet"
            action={
              <Link className="pv-btn pv-btn--primary" to={createHref}>
                Create a voyage
              </Link>
            }
          >
            Be the first. Every voyage on PinVoyager is public.
          </EmptyState>
        ) : (
          <>
            <VoyageGrid desktop={desktop} aria-labelledby={GRID_HEADING_ID}>
              {voyages.map((voyage) => (
                <VoyageCard key={voyage.id} voyage={voyage} desktop={desktop} />
              ))}
            </VoyageGrid>
            <LoadMore
              hasNextPage={feed.hasNextPage}
              isFetchingNextPage={feed.isFetchingNextPage}
              onLoadMore={() => {
                void feed.fetchNextPage();
              }}
            />
          </>
        )}
      </section>
    </div>
  );
}
