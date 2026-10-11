import { Link } from "react-router";

import { isApiError } from "../../api/client";
import {
  Button,
  EmptyState,
  ImageIcon,
  PhotoGrid,
  Skeleton,
  WarningTriangleIcon,
} from "../../components";

import { RAIL_WIDTH, fillHeight } from "./model";
export function LoadingState({ desktop }: { desktop: boolean }) {
  const tiles = [0, 1, 2, 3];

  return (
    <div
      aria-busy="true"
      style={{
        ...fillHeight(desktop),
        display: "grid",
        gridTemplateColumns: desktop ? `1fr ${RAIL_WIDTH}px` : "1fr",
      }}
    >
      <span className="pv-sr">Loading this voyage…</span>
      <div className="pv-map" />
      {desktop && (
        <div
          style={{
            borderLeft: "1px solid var(--pv-border)",
            padding: "var(--pv-space-4)",
            display: "flex",
            flexDirection: "column",
            gap: "var(--pv-space-3)",
          }}
        >
          <Skeleton width="40%" height={12} />
          <Skeleton width="80%" height={24} />
          <Skeleton width="60%" height={12} />
          <PhotoGrid>
            {tiles.map((tile) => (
              <Skeleton key={tile} className="pv-photo pv-photo--square" />
            ))}
          </PhotoGrid>
        </div>
      )}
    </div>
  );
}

export function ErrorState({ error, onRetry }: { error: Error; onRetry: () => void }) {
  const missing = isApiError(error) && error.status === 404;
  if (missing) return <NotFound />;

  const requestId = isApiError(error) ? error.requestId : null;

  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-8)" }}>
      <EmptyState
        tone="danger"
        icon={<WarningTriangleIcon size={40} />}
        title="This voyage could not be loaded"
        action={
          <Button
            variant="secondary"
            onClick={onRetry}
            style={{ minHeight: "var(--pv-touch-min)" }}
          >
            Try again
          </Button>
        }
      >
        Something went wrong at our end.
        {requestId !== null && <> Quote reference {requestId} if you report it.</>}
      </EmptyState>
    </div>
  );
}

export function NotFound() {
  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-8)" }}>
      <EmptyState
        icon={<ImageIcon size={40} />}
        title="This voyage is not here"
        action={
          <Link className="pv-btn pv-btn--secondary" to="/">
            Discover voyages
          </Link>
        }
      >
        It may have been deleted, or it may never have been published.
      </EmptyState>
    </div>
  );
}

export function NoPhotos({ voyageId }: { voyageId: string }) {
  return (
    <EmptyState
      icon={<ImageIcon size={40} />}
      title="No photographs yet"
      action={
        <Link className="pv-btn pv-btn--secondary" to={`/voyages/${voyageId}/edit`}>
          Add photos
        </Link>
      }
    >
      This voyage has nothing in it, so there is nothing to put on the map.
    </EmptyState>
  );
}

/* ==========================================================================
   Formatting
   ========================================================================== */
