/**
 * Public profile — implements `design/screens/profile.html`.
 *
 * A creator and their voyages, readable without an account. Deliberately thin:
 * there is no follower count and no follow button, and there should not be —
 * there is no social graph behind them, and drawing the affordance without the
 * feature would be a promise the backend does not keep.
 *
 * ## Drafts
 *
 * The owner viewing their own profile sees their drafts, each with a Draft
 * badge over the cover. Nobody else sees them at all — not greyed out, not
 * counted, not hinted at. That is the client half of the server's rule that a
 * draft answers 404 rather than 403, so the existence of unpublished work does
 * not leak through a status code or through a gap in a grid. The two halves
 * have to agree or neither is worth anything: the owner's list comes from
 * `?mine=true`, which is the only request that returns a draft at all.
 *
 * ## What the API cannot yet supply, and what this screen does about it
 *
 * There is no endpoint for *another* user. `/api/voyages` takes `mine` and
 * nothing else, and there is no `/api/users/{id}`. So:
 *
 *  * **A stranger's voyages** are the public feed filtered by `owner_id`. That
 *    is correct as far as it goes — the feed returns only published voyages —
 *    but it only sees the pages that have been fetched, which is why the empty
 *    state waits until the feed is exhausted before claiming there is nothing
 *    here.
 *  * **A stranger's name, avatar and join date do not exist on the client.**
 *    The design's header is the creator's name over a count; with no name to
 *    read, this renders the heading as "Voyages" and the avatar as the person
 *    glyph `Avatar` already falls back to. Inventing a name, or deriving one
 *    from the id, would be worse than admitting there is none. The owner's own
 *    profile has `useAuth().user` and shows the real thing.
 *  * **Counts are of what has been loaded**, so they are shown only once the
 *    last page has arrived. A number that silently means "so far" on a
 *    paginated list is a number that is wrong for as long as it is interesting.
 *
 * None of this is a layout decision — the moment `VoyageSummary` carries a
 * creator and there is a user endpoint, the header fills in and the filter
 * becomes a query parameter.
 */

import { Link, useParams } from "react-router";

import { useVoyages } from "../api/hooks";
import { useAuth } from "../auth/useAuth";
import { Avatar, EmptyState, PinIcon } from "../components";
import { useIsDesktop } from "../shell/useIsDesktop";
import {
  LoadMore,
  VoyageCard,
  VoyageGrid,
  VoyageGridSkeleton,
  monthYear,
  referenceOf,
  voyagesOf,
} from "./Discover";
import { ErrorScreen } from "./NotFound";

const HEADING_ID = "pv-profile-name";

export function Profile() {
  const { userId } = useParams<{ userId: string }>();
  const desktop = useIsDesktop();
  const { status, user } = useAuth();

  // Held as the user rather than as a boolean so the header can read the name,
  // the avatar and the join date off it without re-checking for null.
  const owner = user !== null && userId !== undefined && user.id === userId ? user : null;
  const isOwner = owner !== null;

  // `mine=true` is the only request that returns drafts, and it returns the
  // caller's — so it is correct exactly when the caller is the person whose
  // profile this is. For anybody else the public feed is filtered below.
  const feed = useVoyages(isOwner ? { mine: true } : {});

  const loaded = voyagesOf(feed.data);
  const voyages = isOwner ? loaded : loaded.filter((voyage) => voyage.owner_id === userId);

  const drafts = voyages.filter((voyage) => !voyage.published).length;
  const published = voyages.length - drafts;

  // The session is still being read, so we do not yet know whether this is the
  // owner. Showing the public view first and swapping it for the owner's would
  // make a creator's own drafts appear a beat late, which reads as data loss.
  const pending = status === "loading" || feed.isPending;

  const name = owner?.display_name ?? null;

  const summary = summaryLine({
    complete: !pending && !feed.isError && !feed.hasNextPage,
    published,
    // A stranger never sees a draft, and never sees that one was withheld.
    drafts: isOwner ? drafts : 0,
    joined: owner?.created_at ?? null,
    showJoined: desktop,
  });

  return (
    <div>
      <header
        style={{
          padding: desktop
            ? "var(--pv-space-8) var(--pv-space-6)"
            : "var(--pv-space-6) var(--pv-space-4)",
          background: "var(--pv-bg-subtle)",
          borderBottom: "1px solid var(--pv-border)",
          textAlign: desktop ? "start" : "center",
        }}
      >
        {/* `.pv-avatar` is a grid box, so `text-align: center` on the header
            does not centre it — the design file gives it `margin: 0 auto`, and
            `Avatar` takes no style, so the flex row does the same job. */}
        <div
          className={desktop ? "pv-inline-3" : undefined}
          style={
            desktop
              ? { gap: "var(--pv-space-5)" }
              : { display: "flex", justifyContent: "center", marginBottom: "var(--pv-space-3)" }
          }
        >
          <Avatar
            name={name}
            src={owner?.avatar_url ?? null}
            size="lg"
            // With no name beside it there is nothing else identifying whose
            // profile this is, so the avatar needs a name of its own. When the
            // heading names the person, a second announcement is noise.
            label={name === null ? (isOwner ? "Your account" : "This creator") : undefined}
          />
          {desktop && (
            <div>
              <ProfileIdentity
                headingId={HEADING_ID}
                desktop
                name={name}
                summary={summary}
                isOwner={isOwner}
              />
            </div>
          )}
        </div>

        {!desktop && (
          <ProfileIdentity
            headingId={HEADING_ID}
            desktop={false}
            name={name}
            summary={summary}
            isOwner={isOwner}
          />
        )}
      </header>

      <section
        aria-labelledby={HEADING_ID}
        style={{ padding: desktop ? "var(--pv-space-6)" : "var(--pv-space-4)" }}
      >
        {pending ? (
          <VoyageGridSkeleton desktop={desktop} />
        ) : feed.isError ? (
          <ErrorScreen
            title="These voyages could not be loaded"
            reference={referenceOf(feed.error)}
            onRetry={() => {
              void feed.refetch();
            }}
          />
        ) : voyages.length === 0 && !feed.hasNextPage ? (
          isOwner ? (
            <EmptyState
              icon={<PinIcon size={40} strokeWidth={1.5} />}
              title="No voyages yet"
              action={
                <Link className="pv-btn pv-btn--primary" to="/upload">
                  Upload photos
                </Link>
              }
            >
              A voyage is a set of photos on a map. Upload some pictures and PinVoyager will
              place them using the location data your camera recorded.
            </EmptyState>
          ) : (
            <EmptyState icon={<PinIcon size={40} strokeWidth={1.5} />} title="No public voyages">
              This creator has not published anything yet. Every voyage on PinVoyager is
              public once it is published.
            </EmptyState>
          )
        ) : (
          <>
            <VoyageGrid desktop={desktop} aria-labelledby={HEADING_ID}>
              {voyages.map((voyage) => (
                <VoyageCard
                  key={voyage.id}
                  voyage={voyage}
                  desktop={desktop}
                  // Only the owner. A draft is not merely unbadged for a
                  // stranger — it is not in their list at all.
                  draftBadge={isOwner}
                />
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

/**
 * The name, the count line, and the owner's one action.
 *
 * Identical content at both widths — the design only changes where it sits and
 * how large the heading is — so it is written once and placed twice rather than
 * duplicated inside the branch.
 *
 * There is no follow button and no follower count here, and their absence is
 * the design's annotation 2 rather than an omission: there is no social graph
 * in this product, and the affordance without the feature would be a promise
 * the backend does not keep.
 */
function ProfileIdentity({
  headingId,
  desktop,
  name,
  summary,
  isOwner,
}: {
  headingId: string;
  desktop: boolean;
  name: string | null;
  summary: string;
  isOwner: boolean;
}) {
  return (
    <>
      <h1 className={desktop ? "pv-h2" : "pv-h3"} id={headingId}>
        {name ?? "Voyages"}
      </h1>
      {summary !== "" && (
        <p
          className={desktop ? "pv-sm pv-muted" : "pv-xs pv-muted"}
          style={{ marginTop: "var(--pv-space-2)" }}
        >
          {summary}
        </p>
      )}
      {isOwner && (
        <Link
          className="pv-btn pv-btn--primary pv-btn--lg"
          to="/upload"
          style={{
            marginTop: "var(--pv-space-4)",
            // Full width on a phone, as the design draws it; a 48px-tall
            // button spanning the card is the one action on that screen.
            ...(desktop ? {} : { width: "100%" }),
          }}
        >
          New voyage
        </Link>
      )}
    </>
  );
}

/**
 * "6 voyages · 1 draft · joined February 2026".
 *
 * Empty until the last page has arrived, because until then every count is a
 * count of what happens to have been fetched. The design shows totals; a
 * cursor-paginated list cannot produce one without a `total` on the response.
 */
function summaryLine({
  complete,
  published,
  drafts,
  joined,
  showJoined,
}: {
  complete: boolean;
  published: number;
  drafts: number;
  joined: string | null;
  showJoined: boolean;
}): string {
  if (!complete) return "";

  const parts = [`${published} ${published === 1 ? "voyage" : "voyages"}`];
  if (drafts > 0) parts.push(`${drafts} ${drafts === 1 ? "draft" : "drafts"}`);
  if (showJoined && joined !== null) parts.push(`joined ${monthYear(joined)}`);

  return parts.join(" · ");
}
