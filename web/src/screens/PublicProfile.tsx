/** Published voyages resolved by account ID, current handle, or historical alias. */
import { Navigate, useParams } from "react-router";
import { isApiError } from "../api/client";
import { useAuth } from "../auth/useAuth";
import { OwnerVoyages } from "./OwnerVoyages";
import { usePublicProfile } from "../api/hooks";
import { Avatar, EmptyState } from "../components";
import { useIsDesktop } from "../shell/useIsDesktop";
import { LoadMore, VoyageCard, VoyageGrid } from "../components/VoyageCards";
import { referenceOf } from "../components/voyagePresentation";
import { ErrorScreen } from "./NotFound";

export function PublicProfile() {
  const { identifier } = useParams<{ identifier: string }>();
  const feed = usePublicProfile(identifier);
  const desktop = useIsDesktop();
  const auth = useAuth();
  if (feed.isPending)
    return (
      <p className="pv-container pv-muted" role="status">
        Loading profile…
      </p>
    );
  if (feed.isError) {
    return isApiError(feed.error) && feed.error.status === 404 ? (
      <EmptyState title="Profile not found">No account matches this name or ID.</EmptyState>
    ) : (
      <ErrorScreen
        title="This profile could not be loaded"
        reference={referenceOf(feed.error)}
        onRetry={() => {
          void feed.refetch();
        }}
      />
    );
  }
  const pages = feed.data!.pages;
  const user = pages[0]!.user;
  if (user.display_name !== null && user.display_name !== identifier && user.id !== identifier)
    return <Navigate to={`/u/${user.display_name}`} replace />;
  if (auth.user?.id === user.id) return <OwnerVoyages owner={user} desktop={desktop} />;
  const voyages = pages.flatMap((page) => page.voyages);
  return (
    <div className="pv-container" style={{ paddingBlock: "var(--pv-space-6)" }}>
      <header className="pv-inline-3" style={{ marginBottom: "var(--pv-space-6)" }}>
        <Avatar name={user.display_name} src={user.avatar_url} size="lg" />
        <h1 className="pv-h2">{user.display_name ?? "Voyages"}</h1>
      </header>
      {voyages.length === 0 && !feed.hasNextPage ? (
        <EmptyState title="No public voyages">
          This creator has not published any voyages yet.
        </EmptyState>
      ) : (
        <VoyageGrid desktop={desktop}>
          {voyages.map((voyage) => (
            <VoyageCard key={voyage.id} voyage={voyage} desktop={desktop} />
          ))}
        </VoyageGrid>
      )}
      <LoadMore
        hasNextPage={feed.hasNextPage}
        isFetchingNextPage={feed.isFetchingNextPage}
        onLoadMore={() => {
          void feed.fetchNextPage();
        }}
      />
    </div>
  );
}
