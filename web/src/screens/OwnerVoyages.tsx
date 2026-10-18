/** The signed-in account's voyages, including drafts and creation controls. */
import { Link } from "react-router";
import { useVoyages } from "../api/hooks";
import type { CurrentUser } from "../api/types";
import { Avatar, EmptyState, PinIcon } from "../components";
import { LoadMore, VoyageCard, VoyageGrid, VoyageGridSkeleton } from "../components/VoyageCards";
import { monthYear, referenceOf, voyagesOf } from "../components/voyagePresentation";
import { ErrorScreen } from "./NotFound";

export function OwnerVoyages({
  owner,
  desktop,
}: {
  owner: Omit<CurrentUser, "is_site_admin">;
  desktop: boolean;
}) {
  const feed = useVoyages({ mine: true });
  const voyages = voyagesOf(feed.data);
  const drafts = voyages.filter((voyage) => !voyage.published).length;
  const published = voyages.length - drafts;
  const summary =
    !feed.isPending && !feed.isError && !feed.hasNextPage
      ? [
          `${published} ${published === 1 ? "voyage" : "voyages"}`,
          ...(drafts ? [`${drafts} ${drafts === 1 ? "draft" : "drafts"}`] : []),
          ...(desktop ? [`joined ${monthYear(owner.created_at)}`] : []),
        ].join(" · ")
      : "";
  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-6)" }}>
      <header className="pv-stack-3" style={{ marginBottom: "var(--pv-space-6)" }}>
        <Avatar
          name={owner.display_name}
          src={owner.avatar_url}
          size="lg"
          label={owner.display_name === null ? "Your account" : undefined}
        />
        <h1 className="pv-h2" id="pv-profile-name">
          {owner.display_name ?? "My voyages"}
        </h1>
        {summary && <p className="pv-muted">{summary}</p>}
        <Link className="pv-btn pv-btn--primary" to="/upload">
          New voyage
        </Link>
      </header>
      {feed.isPending ? (
        <VoyageGridSkeleton desktop={desktop} />
      ) : feed.isError ? (
        <ErrorScreen
          title="These voyages could not be loaded"
          reference={referenceOf(feed.error)}
          onRetry={() => void feed.refetch()}
        />
      ) : voyages.length === 0 && !feed.hasNextPage ? (
        <EmptyState
          icon={<PinIcon size={40} />}
          title="No voyages yet"
          action={
            <Link className="pv-btn pv-btn--primary" to="/upload">
              Upload photos
            </Link>
          }
        >
          A voyage is a set of photos on a map. Upload pictures to start one.
        </EmptyState>
      ) : (
        <>
          <VoyageGrid desktop={desktop} aria-labelledby="pv-profile-name">
            {voyages.map((voyage) => (
              <VoyageCard key={voyage.id} voyage={voyage} desktop={desktop} draftBadge />
            ))}
          </VoyageGrid>
          <LoadMore
            hasNextPage={feed.hasNextPage}
            isFetchingNextPage={feed.isFetchingNextPage}
            onLoadMore={() => void feed.fetchNextPage()}
          />
        </>
      )}
    </div>
  );
}
