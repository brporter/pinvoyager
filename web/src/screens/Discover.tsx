/** Public discovery map and paginated published-voyage cards. */

import { Link } from "react-router";
import { useVoyages } from "../api/hooks";
import { signInPath } from "../auth/redirect";
import { useAuth } from "../auth/useAuth";
import { EmptyState, SearchIcon } from "../components";
import { LoadMore, VoyageCard, VoyageGrid, VoyageGridSkeleton } from "../components/VoyageCards";
import { voyagesOf, referenceOf } from "../components/voyagePresentation";
import { LazyDiscoverMap } from "../map";
import { useIsDesktop } from "../shell/useIsDesktop";
import { ErrorScreen } from "./NotFound";

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
              Upload your pictures and PinVoyager reads the location your camera recorded, putting
              each one on a map. You choose how precise that is.
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

      {/* The map shares this page with the paginated voyage grid. */}
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
