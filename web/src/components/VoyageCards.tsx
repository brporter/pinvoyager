import { monthYear } from "./voyagePresentation";
import type { CSSProperties, ReactNode } from "react";
import { Link } from "react-router";
import type { VoyageSummary } from "../api/types";
import { Badge, Button, Photo, Skeleton } from "./index";
/** VoyageCards uses the shared runtime styles and accessible native controls. */
const FLUSH_COVER = {
  "--pv-radius-lg": "0px",
  "--pv-radius-md": "0px",
} as unknown as CSSProperties;

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
  /** Drawn over the cover. Only the
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
          <Photo
            src={voyage.cover_url}
            badge={draft ? <Badge tone="draft">Draft</Badge> : undefined}
          />
        </div>

        <div className="pv-voyage-card-body">
          <h3
            className="pv-voyage-card-title"
            style={desktop ? undefined : { fontSize: "var(--pv-text-lg)" }}
          >
            {voyage.title}
          </h3>
          <div className="pv-voyage-card-meta">
            <span>
              {draft ? "Not published" : monthYear(voyage.published_at ?? voyage.created_at)}
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
      <Button variant="secondary" size="lg" loading={isFetchingNextPage} onClick={onLoadMore}>
        Load more voyages
      </Button>
    </div>
  );
}
