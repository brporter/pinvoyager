//! Voyages repository.
//!
//! Functions accept executors so handlers can combine operations in a
//! transaction and tests can roll back their changes.
//!
//! Repository coordinates retain stored precision. Public response construction
//! applies granularity; these row types do not implement `Serialize`.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Voyage {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub published: bool,
    /// Stored spelling; parse with `Granularity::parse`.
    pub granularity: String,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

/// A photo as it appears inside a voyage. Coordinates are **exact** here.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VoyagePhoto {
    pub photo_id: Uuid,
    pub position: i32,
    pub captured_at: Option<DateTime<Utc>>,
    pub width: i32,
    pub height: i32,
    pub variant_status: String,
    /// Exact stored latitude. Round before including in a public response.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct NewVoyage {
    pub owner_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub granularity: String,
}

/// Fields a PATCH may change. `None` means "leave alone", which is why this is
/// not just a `Voyage` — a struct of plain values cannot distinguish "set the
/// description to null" from "do not touch the description".
#[derive(Debug, Clone, Default)]
pub struct VoyagePatch {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub granularity: Option<String>,
    pub published: Option<bool>,
}

// Checked queries spell out the selected columns. `query_as!` maps their
// names to struct fields; update the queries and offline metadata when the
// row type changes.

pub async fn create(
    executor: impl PgExecutor<'_>,
    voyage: &NewVoyage,
) -> Result<Voyage, sqlx::Error> {
    // Generate a time-ordered UUIDv7 for owner-list pagination.
    let id = Uuid::now_v7();

    // New voyages default to unpublished; publication is a separate update.
    // Unsupported granularity strings return a database CHECK-constraint error.
    sqlx::query_as!(
        Voyage,
        "INSERT INTO voyages (id, owner_id, title, description, granularity)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, owner_id, title, description, published, granularity,
                   created_at, published_at",
        id,
        voyage.owner_id,
        voyage.title,
        voyage.description,
        voyage.granularity,
    )
    .fetch_one(executor)
    .await
}

pub async fn find_by_id(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> Result<Option<Voyage>, sqlx::Error> {
    // Visibility is checked by the HTTP handler; this lookup is not owner-scoped.
    sqlx::query_as!(
        Voyage,
        "SELECT id, owner_id, title, description, published, granularity,
                created_at, published_at
           FROM voyages
          WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await
}

/// Owner-scoped update. `Ok(None)` for "no such voyage, or not yours" — the two
/// are deliberately indistinguishable, and the ownership predicate lives in the
/// `WHERE` clause so there is no window between a check and the write.
///
/// Sets `published_at` on the first transition to published, and leaves it
/// alone afterwards: the discovery feed orders by when something first became
/// visible, and unpublishing then republishing should not jump the queue.
pub async fn update(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner_id: Uuid,
    patch: &VoyagePatch,
) -> Result<Option<Voyage>, sqlx::Error> {
    // Bind description presence separately so omission preserves the value
    // and an explicit null clears it.
    let description_given = patch.description.is_some();
    let description = patch.description.as_ref().and_then(Option::as_deref);

    // Apply all supplied fields in one statement.
    // Set `published_at` only on the first publication; preserve it when
    // unpublishing or republishing.
    // Explicit casts give nullable parameters their SQL types.
    sqlx::query_as!(
        Voyage,
        "UPDATE voyages
            SET title        = COALESCE($3::text, title),
                description  = CASE WHEN $4::boolean THEN $5::text ELSE description END,
                granularity  = COALESCE($6::text, granularity),
                published    = COALESCE($7::boolean, published),
                published_at = CASE
                                   WHEN COALESCE($7::boolean, published)
                                        AND published_at IS NULL
                                   THEN now()
                                   ELSE published_at
                               END,
                updated_at   = now()
          WHERE id = $1 AND owner_id = $2
         RETURNING id, owner_id, title, description, published, granularity,
                   created_at, published_at",
        id,
        owner_id,
        patch.title.as_deref(),
        description_given,
        description,
        patch.granularity.as_deref(),
        patch.published,
    )
    .fetch_optional(executor)
    .await
}

/// Owner-scoped delete. Returns whether anything was removed.
///
/// Deleting a voyage cascades to its photo records (D11).
pub async fn delete(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner_id: Uuid,
) -> Result<bool, sqlx::Error> {
    // `photos_voyage_owner_fkey` deletes the voyage's photo rows. The HTTP
    // handler records their content hashes for cleanup in this transaction;
    // cleanup removes unreferenced files after commit.
    let result = sqlx::query!(
        "DELETE FROM voyages WHERE id = $1 AND owner_id = $2",
        id,
        owner_id,
    )
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Internal reassignment of already voyage-owned records; no HTTP attachment API.
pub async fn move_photos(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
    owner_id: Uuid,
    photo_ids: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "WITH candidates AS (
            SELECT id, (COALESCE((SELECT max(position) FROM photos WHERE voyage_id = $1), 0)
                + row_number() OVER (ORDER BY captured_at NULLS LAST, id))::int AS position
            FROM photos WHERE owner_id = $2 AND id = ANY($3::uuid[]) AND voyage_id <> $1
        ) UPDATE photos p SET voyage_id = $1, position = c.position FROM candidates c
          WHERE p.id = c.id AND p.voyage_id <> $1 AND EXISTS (SELECT 1 FROM voyages WHERE id = $1 AND owner_id = $2)",
        voyage_id,
        owner_id,
        photo_ids,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

pub async fn detach_photo(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
    owner_id: Uuid,
    photo_id: Uuid,
) -> Result<bool, sqlx::Error> {
    // Return false for a missing voyage, a foreign voyage, or a photo outside it.
    let result = sqlx::query!(
        "DELETE FROM voyage_photos
          WHERE voyage_id = $1
            AND photo_id = $3
            AND EXISTS (SELECT 1 FROM voyages v WHERE v.id = $1 AND v.owner_id = $2)",
        voyage_id,
        owner_id,
        photo_id,
    )
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Reorders voyage photos and returns database errors.
///
/// Known IDs come first in request order; the first occurrence of a duplicate
/// wins. Unknown IDs are ignored. Omitted photos follow in their previous order,
/// with photo ID breaking ties. Positions are renumbered from one. A missing
/// or foreign voyage changes no rows.
pub async fn reorder(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
    owner_id: Uuid,
    photo_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    // Update every position in one statement so readers see a complete order.
    // Start with all voyage photos. Requested ordinality comes first, followed
    // by omitted photos ordered by their previous position and ID.
    // Keep the first occurrence of each requested ID to avoid multiple ranking
    // rows for one photo.
    // The SQL ownership check supplements the handler's owner check.
    sqlx::query!(
        "UPDATE voyage_photos vp
            SET position = ranked.position
           FROM (
                SELECT existing.photo_id,
                       (row_number() OVER (ORDER BY desired.ordinality NULLS LAST,
                                                    existing.position,
                                                    existing.photo_id))::int AS position
                  FROM voyage_photos existing
                  LEFT JOIN (
                            SELECT DISTINCT ON (photo_id) photo_id, ordinality
                              FROM unnest($3::uuid[]) WITH ORDINALITY AS d(photo_id, ordinality)
                             ORDER BY photo_id, ordinality
                       ) AS desired ON desired.photo_id = existing.photo_id
                 WHERE existing.voyage_id = $1
           ) AS ranked
          WHERE vp.voyage_id = $1
            AND vp.photo_id = ranked.photo_id
            AND EXISTS (SELECT 1 FROM voyages v WHERE v.id = $1 AND v.owner_id = $2)",
        voyage_id,
        owner_id,
        photo_ids,
    )
    .execute(executor)
    .await?;

    Ok(())
}

/// Returns all voyage photos ordered by position and photo ID in one query.
pub async fn photos_for_voyage(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
) -> Result<Vec<VoyagePhoto>, sqlx::Error> {
    // Photo ID breaks position ties deterministically.
    // Convert stored decimal coordinates to the row type's `f64` fields.
    // Public response construction applies granularity separately.
    sqlx::query_as!(
        VoyagePhoto,
        r#"SELECT p.id AS "photo_id!",
                  p.position,
                  p.captured_at,
                  p.width,
                  p.height,
                  p.variant_status,
                  p.latitude::float8  AS latitude,
                  p.longitude::float8 AS longitude
             FROM photos p
            WHERE p.voyage_id = $1
            ORDER BY p.position, p.id"#,
        voyage_id,
    )
    .fetch_all(executor)
    .await
}

/// A stable tie-breaker prevents equal publication timestamps from losing rows.
#[derive(Debug, Clone, Copy)]
pub struct PublishedCursor {
    pub at: DateTime<Utc>,
    pub id: Uuid,
}
impl PublishedCursor {
    pub fn parse(value: &str) -> Option<Self> {
        let (at, id) = value.rsplit_once('|')?;
        Some(Self {
            at: DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc),
            id: id.parse().ok()?,
        })
    }
    pub fn of(voyage: &Voyage) -> Option<Self> {
        voyage.published_at.map(|at| Self { at, id: voyage.id })
    }
    pub fn encode(self) -> String {
        format!("{}|{}", self.at.to_rfc3339(), self.id)
    }
}

/// Public feed ordered by publication time and ID, both descending.
pub async fn list_published(
    executor: impl PgExecutor<'_>,
    before: Option<PublishedCursor>,
    limit: i64,
) -> Result<Vec<Voyage>, sqlx::Error> {
    list_published_visible(executor, before, limit, None).await
}

pub async fn list_published_visible(
    executor: impl PgExecutor<'_>,
    before: Option<PublishedCursor>,
    limit: i64,
    viewer: Option<Uuid>,
) -> Result<Vec<Voyage>, sqlx::Error> {
    sqlx::query_as("SELECT id, owner_id, title, description, published, granularity, created_at, published_at FROM voyages WHERE published AND NOT moderation_held AND NOT EXISTS(SELECT 1 FROM account_blocks b WHERE b.account_id=$4 AND b.blocked_id=voyages.owner_id) AND published_at IS NOT NULL AND ($1::timestamptz IS NULL OR (published_at,id) < ($1,$2)) ORDER BY published_at DESC, id DESC LIMIT $3")
        .bind(before.map(|c| c.at)).bind(before.map(|c| c.id)).bind(limit).bind(viewer).fetch_all(executor).await
}

/// A user's own voyages, drafts included.
pub async fn list_for_owner(
    executor: impl PgExecutor<'_>,
    owner_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Voyage>, sqlx::Error> {
    // Keyset on the v7 id rather than on a timestamp, matching
    // `photos::list_for_owner`: the id is time-ordered, so `id < $2` means
    // "older than that voyage", it is unique so pages tile exactly with no
    // tiebreak needed, and `voyages_owner_id_idx` on `(owner_id, id DESC)`
    // serves the filter and the sort together.
    //
    // The cast on `$2` is required — a parameter that only appears next to
    // `IS NULL` gives Postgres nothing to infer from.
    sqlx::query_as!(
        Voyage,
        "SELECT id, owner_id, title, description, published, granularity,
                created_at, published_at
           FROM voyages
          WHERE owner_id = $1
            AND ($2::uuid IS NULL OR id < $2)
          ORDER BY id DESC
          LIMIT $3",
        owner_id,
        before,
        limit,
    )
    .fetch_all(executor)
    .await
}

/// One placed photograph in a published voyage, for the discovery map.
///
/// Coordinates are **exact**, like every other row type here, and the
/// granularity travels with them as the stored string so that the rounding
/// still happens once at the serialization boundary rather than in this query.
/// Every photo belongs to one voyage; separate uploads of identical bytes
/// remain separate photos and may use different voyage granularities.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DiscoverPhoto {
    pub voyage_title: String,
    pub photo_id: Uuid,
    pub voyage_id: Uuid,
    /// Stored spelling; parse with `Granularity::parse`.
    pub granularity: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Scan public candidates in bounded ID pages. Exact coordinates never decide
/// membership or truncation: the HTTP layer applies granularity first.
pub async fn published_photo_page_visible(
    executor: impl PgExecutor<'_>,
    after: Option<Uuid>,
    limit: i64,
    viewer: Option<Uuid>,
) -> Result<Vec<DiscoverPhoto>, sqlx::Error> {
    sqlx::query_as("SELECT p.id AS photo_id, v.id AS voyage_id, v.title AS voyage_title, v.granularity, p.latitude::float8 AS latitude, p.longitude::float8 AS longitude FROM photos p JOIN voyages v ON v.id=p.voyage_id WHERE v.published AND NOT v.moderation_held AND NOT EXISTS(SELECT 1 FROM account_blocks b WHERE b.account_id=$3 AND b.blocked_id=v.owner_id) AND p.latitude IS NOT NULL AND ($1::uuid IS NULL OR p.id > $1) ORDER BY p.id LIMIT $2")
        .bind(after).bind(limit).bind(viewer).fetch_all(executor).await
}

/// Cover metadata for a page, in one query rather than one query per card.
#[derive(Debug)]
pub struct VoyageCover {
    pub voyage_id: Uuid,
    pub photo_id: Option<Uuid>,
    pub pending: bool,
}

pub async fn covers_for_voyages(
    executor: impl PgExecutor<'_>,
    voyage_ids: &[Uuid],
) -> Result<Vec<VoyageCover>, sqlx::Error> {
    sqlx::query_as!(
        VoyageCover,
        r#"SELECT voyage_id,
                  (array_agg(id ORDER BY position, id)
                      FILTER (WHERE variant_status = 'ready'))[1] AS photo_id,
                  bool_or(variant_status = 'pending') AS "pending!"
             FROM photos
            WHERE voyage_id = ANY($1::uuid[])
            GROUP BY voyage_id"#,
        voyage_ids,
    )
    .fetch_all(executor)
    .await
}

/// Public profile pagination applies ownership in SQL, before the limit.
pub async fn list_published_for_owner(
    executor: impl PgExecutor<'_>,
    owner_id: Uuid,
    before: Option<PublishedCursor>,
    limit: i64,
) -> Result<Vec<Voyage>, sqlx::Error> {
    sqlx::query_as("SELECT id, owner_id, title, description, published, granularity, created_at, published_at FROM voyages WHERE published AND NOT moderation_held AND owner_id=$1 AND published_at IS NOT NULL AND ($2::timestamptz IS NULL OR (published_at,id) < ($2,$3)) ORDER BY published_at DESC, id DESC LIMIT $4")
        .bind(owner_id).bind(before.map(|c| c.at)).bind(before.map(|c| c.id)).bind(limit).fetch_all(executor).await
}

#[cfg(test)]
#[path = "voyages_tests.rs"]
mod tests;
