//! Voyages repository.
//!
//! CONTRACT — signatures are fixed; implementation owned by the CRUD agent.
//!
//! Every function takes an executor, so a handler can create a voyage and
//! attach photos in one transaction and tests can roll back.
//!
//! **Nothing here applies granularity.** These types carry exact coordinates
//! and deliberately do not implement `Serialize`; the rounding happens once, at
//! the serialization boundary in `http::voyages`. Putting it here would spread
//! the decision across every query and defeat the point.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct VoyagePhoto {
    pub photo_id: Uuid,
    pub position: i32,
    pub captured_at: Option<DateTime<Utc>>,
    pub width: i32,
    pub height: i32,
    pub variant_status: String,
    /// Exact, straight from the column. Must pass through
    /// `Granularity::apply` before it can be serialized.
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

// Every query below selects the same column list, spelled out in full each
// time. `query_as!` needs a string literal — it reads the SQL at compile time to
// check it against the database, so a `concat!` or a helper macro is not
// something it can see through. The list is also positional: `query_as!` maps
// columns to `Voyage`'s fields in order, so the four copies and the struct have
// to be changed together.

pub async fn create(
    executor: impl PgExecutor<'_>,
    voyage: &NewVoyage,
) -> Result<Voyage, sqlx::Error> {
    // v7 for the same reason as `photos::create`: the ids are time-ordered, so
    // `voyages_owner_id_idx` — declared `(owner_id, id DESC)` — serves "my
    // voyages, newest first" as an index scan with no sort, and the caller
    // holds the id before the round trip completes.
    let id = Uuid::now_v7();

    // `published` is left to its default of false. T8: a voyage must not be
    // world-readable the instant it is created and half-populated, so
    // publishing is a separate, deliberate act (see `update`).
    //
    // An unknown `granularity` spelling surfaces as a CHECK violation rather
    // than being validated here. The migration owns the permitted set; a second
    // copy of the list in Rust is a second thing to forget to update.
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
    // Not owner-scoped: a published voyage is world-readable (D6) and this is
    // the lookup the public detail endpoint uses. Draft visibility is the
    // handler's decision — it has the caller's identity and it owes a 404
    // rather than a 403, so that probing ids does not confirm that a draft
    // exists.
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
    // `description` needs two parameters where the others need one. `COALESCE`
    // reads "keep the old value when the new one is null", which is exactly
    // wrong for a field whose new value may legitimately *be* null — the whole
    // reason `VoyagePatch::description` is `Option<Option<String>>`. So the
    // presence of the outer `Option` travels as its own boolean and the CASE
    // switches on that, leaving the inner `None` free to mean "set to null".
    let description_given = patch.description.is_some();
    let description = patch.description.as_ref().and_then(Option::as_deref);

    // One statement, so a PATCH that both publishes and retitles cannot land
    // half-applied, and so there is no read-modify-write window in which a
    // concurrent PATCH's changes are silently reverted.
    //
    // The `published_at` CASE is the rule in the doc comment, written as a
    // predicate rather than as branching Rust: it fires only when the row is
    // ending up published *and* has never been published before. Unpublishing
    // leaves the timestamp in place, so republishing finds it non-null and does
    // not touch it — the voyage returns to the feed at its original position
    // instead of jumping to the front. Nothing here ever clears it.
    //
    // The casts on the parameters are required: a bare `$3` that only ever
    // appears inside `COALESCE(..., title)` gives Postgres nothing to infer a
    // type from, the same reason `photos::list_for_owner` writes `$2::uuid`.
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
/// The join rows cascade; the photos do not. Deleting a voyage returns its
/// photographs to the owner's library.
pub async fn delete(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner_id: Uuid,
) -> Result<bool, sqlx::Error> {
    // The cascade that matters is the one the migration declares on
    // `voyage_photos.voyage_id`. There is deliberately no cascade from here to
    // `photos`: tidying up a draft is not a request to destroy the
    // photographs in it.
    let result = sqlx::query!(
        "DELETE FROM voyages WHERE id = $1 AND owner_id = $2",
        id,
        owner_id,
    )
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Attaches photos the caller owns. Returns how many were added.
///
/// **Must verify ownership of every photo in the same statement**, not in a
/// preceding SELECT: a check-then-insert lets a caller attach someone else's
/// photograph by racing the two. An `INSERT ... SELECT ... WHERE owner_id = $x`
/// makes the ownership predicate part of the write.
///
/// A photo already in the voyage is *not* counted, because it was not added.
/// A handler enforcing T3 — "attaching another user's photo returns 403 and
/// attaches nothing" — should call this inside a transaction and roll back when
/// the count is below `photo_ids.len()`, which also refuses a request that
/// re-attaches something already present. That is the safe direction: the
/// alternative silently half-honors a request the caller believes was applied
/// in full.
pub async fn attach_photos(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
    owner_id: Uuid,
    photo_ids: &[Uuid],
) -> Result<u64, sqlx::Error> {
    // Three predicates, all of them part of the write rather than a check that
    // precedes it:
    //
    // * `candidate.owner_id = $2` — the photo is the caller's. This is the one
    //   the doc comment is about. A `SELECT` that verified ownership and an
    //   `INSERT` that trusted the result are two snapshots with a window
    //   between them; one statement has no window.
    // * `candidate.id = ANY($3)` — and it is one of the ones asked for. An id
    //   the caller does not own simply matches nothing, so a stranger's photo
    //   is not attached and no error distinguishes "not yours" from "no such
    //   photo".
    // * `EXISTS (... voyages ...)` — the *voyage* is the caller's too.
    //   Otherwise anyone could stock a voyage they do not own with their own
    //   photographs.
    //
    // Position defaults to capture time (T4), assigned by `row_number()` over
    // the whole batch so that attaching fifty photos at once orders them among
    // themselves rather than by whatever order the caller's array happened to
    // be in. `NULLS LAST` puts photos with no EXIF timestamp — screenshots,
    // scans — after the dated ones instead of ahead of them, and the `id`
    // tiebreak keeps that group in upload order and makes the whole ordering
    // total, so the same batch always lands the same way.
    //
    // The `max(position)` subquery appends after whatever is already there. It
    // is evaluated against the snapshot taken before the statement ran and
    // therefore cannot see the rows this very statement is inserting — the same
    // property that `photos::delete_returning_orphaned` has to work *around*,
    // and here it is exactly what is wanted: one base, plus a row number.
    //
    // `ON CONFLICT DO NOTHING` makes re-attaching idempotent. It leaves gaps in
    // the position sequence when it fires, since the skipped row still consumed
    // a `row_number()`. Gaps are harmless: every reader orders by position and
    // none of them assume it is contiguous.
    let result = sqlx::query!(
        "INSERT INTO voyage_photos (voyage_id, photo_id, position)
         SELECT $1,
                candidate.id,
                (COALESCE((SELECT max(vp.position)
                             FROM voyage_photos vp
                            WHERE vp.voyage_id = $1), 0)
                 + row_number() OVER (ORDER BY candidate.captured_at ASC NULLS LAST,
                                               candidate.id ASC))::int
           FROM photos AS candidate
          WHERE candidate.owner_id = $2
            AND candidate.id = ANY($3::uuid[])
            AND EXISTS (SELECT 1 FROM voyages v WHERE v.id = $1 AND v.owner_id = $2)
         ON CONFLICT (voyage_id, photo_id) DO NOTHING",
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
    // Ownership is the voyage's, not the photo's: a photo can be removed from a
    // voyage by the person whose voyage it is. The predicate is in the `WHERE`
    // clause for the usual reason — `false` covers "no such voyage", "not
    // yours", and "that photo was not in it" alike, so a caller learns nothing
    // by probing.
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

/// Sets an explicit order. The slice is the desired sequence.
///
/// **Ids not in the voyage are ignored, and photos the caller omitted keep
/// their relative order at the end.** Both cases are ordinary rather than
/// exceptional. The client sends the list it rendered, and between that render
/// and this call a photo can be attached by another tab or detached by the
/// same one; rejecting the whole reorder because the list is one row stale
/// turns a routine race into a visible failure. Ignoring unknown ids also keeps
/// this consistent with every other function here: a foreign photo id behaves
/// exactly like an id that does not exist, so reorder is not an oracle for
/// which photos are real.
///
/// The alternative — leaving omitted photos at their old positions — is worse
/// than it looks: the listed photos are renumbered from 1, so an omitted photo
/// would keep a number that now collides with a listed one and the two would
/// order arbitrarily. Renumbering everything makes the result total and
/// gap-free whatever the caller sent.
pub async fn reorder(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
    owner_id: Uuid,
    photo_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    // One statement, so the voyage is never observably half-reordered. Note
    // that a partial reorder would not violate a constraint even if it did
    // happen — `position` has no unique index, the primary key is
    // `(voyage_id, photo_id)` and this statement does not touch either column —
    // which is precisely why atomicity has to be arranged rather than relied on
    // to fail loudly. The failure mode of doing this row by row is a voyage
    // that reads in a nonsensical order, not an error.
    //
    // The ranking is a `LEFT JOIN` from the voyage's rows to the caller's list,
    // not the other way round: driving from `voyage_photos` is what makes the
    // result cover every photo in the voyage rather than only the mentioned
    // ones. `ordinality` is null for an unmentioned photo, `NULLS LAST` sends
    // it to the back, and the previous `position` keeps that tail in the order
    // it already had. `photo_id` breaks any remaining tie so the outcome is
    // deterministic. `row_number()` over that order yields 1..n with no gaps.
    //
    // The `DISTINCT ON` is not decoration. A repeated id in the caller's list
    // would make the join produce two rows for one photo, and `UPDATE ... FROM`
    // with a multiply-matching join does not error in Postgres — it picks one
    // of the candidate rows arbitrarily and shifts every subsequent row number.
    // Keeping the first occurrence of each id makes a duplicate mean "the
    // earlier place in the list", which is the only reading that is not a
    // surprise.
    //
    // Like the rest of the file the ownership predicate is in the statement.
    // The signature has no way to report "not yours", so this is defense in
    // depth under a handler that has already established ownership, not the
    // handler's only check — see the note in the module's report.
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

/// Every photo in a voyage, in order, in **one** query.
///
/// One query is a requirement, not a preference: `await` inside a loop looks
/// concurrent and is not, and the cost would scale with photo count on the
/// product's most-visited endpoint.
pub async fn photos_for_voyage(
    executor: impl PgExecutor<'_>,
    voyage_id: Uuid,
) -> Result<Vec<VoyagePhoto>, sqlx::Error> {
    // A join, served by `voyage_photos_order_idx` on `(voyage_id, position)`,
    // so the ordering is an index scan rather than a sort. The `photo_id`
    // tiebreak costs nothing and buys a total order: two photos can share a
    // position if concurrent `attach_photos` calls computed the same base, and
    // a voyage whose photos swap places between two reads for no visible reason
    // is a bug nobody would think to look for.
    //
    // The coordinates are cast to `float8` because `VoyagePhoto` declares them
    // as `f64` while the column is `numeric(9, 6)`. Six decimal places is well
    // inside what a double represents exactly enough to print back identically,
    // so nothing is lost — and nothing is *rounded*, which is the point: these
    // are exact coordinates and the granularity transform happens once, at the
    // serialization boundary.
    sqlx::query_as!(
        VoyagePhoto,
        r#"SELECT vp.photo_id,
                  vp.position,
                  p.captured_at,
                  p.width,
                  p.height,
                  p.variant_status,
                  p.latitude::float8  AS latitude,
                  p.longitude::float8 AS longitude
             FROM voyage_photos vp
             JOIN photos p ON p.id = vp.photo_id
            WHERE vp.voyage_id = $1
            ORDER BY vp.position, vp.photo_id"#,
        voyage_id,
    )
    .fetch_all(executor)
    .await
}

/// The discovery feed: published voyages, newest first, cursor-paginated.
pub async fn list_published(
    executor: impl PgExecutor<'_>,
    before: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<Voyage>, sqlx::Error> {
    // Shaped for `voyages_published_idx`, which is
    // `(published_at DESC) WHERE published`. The bare `WHERE published` is what
    // lets the planner match the index's own predicate; writing it as
    // `published = true` or folding it into an `OR` would leave the partial
    // index unusable and turn the feed into a sequential scan over every draft
    // in the table.
    //
    // Keyset, not `OFFSET`, and that is what satisfies T7. A voyage published
    // during a traversal gets `published_at = now()`, which is newer than every
    // cursor already passed, so it can only ever have belonged on a page the
    // reader has already seen — it never shifts the remaining rows and never
    // causes a repeat. `OFFSET` would push one row of every subsequent page
    // past the boundary and the reader would silently miss it.
    //
    // `published_at IS NOT NULL` guards against a row that is published with no
    // timestamp, which `update` cannot produce but a manual fix-up could. Under
    // `DESC` a null sorts first, so such a row would pin itself to the top of
    // the feed forever and be unreachable by any cursor. Excluding it is the
    // honest outcome: a voyage with no publication time has no place in an
    // ordering by publication time.
    sqlx::query_as!(
        Voyage,
        "SELECT id, owner_id, title, description, published, granularity,
                created_at, published_at
           FROM voyages
          WHERE published
            AND published_at IS NOT NULL
            AND ($1::timestamptz IS NULL OR published_at < $1)
          ORDER BY published_at DESC
          LIMIT $2",
        before,
        limit,
    )
    .fetch_all(executor)
    .await
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

/// Is this photograph visible to the public — that is, attached to at least one
/// **published** voyage?
///
/// This is what makes D6 real rather than nominal. Without it, every voyage is
/// public and every image inside it 404s for anyone but the owner: the JSON is
/// world-readable and the photographs are not, which is a public gallery with
/// the pictures painted out.
///
/// A draft does not count. Attaching a photo to an unpublished voyage must not
/// make it fetchable, or "draft" would leak its contents to anyone holding a
/// photo id.
pub async fn photo_is_publicly_visible(
    executor: impl PgExecutor<'_>,
    photo_id: Uuid,
) -> Result<bool, sqlx::Error> {
    // `EXISTS` rather than a count: the answer is a boolean and the planner can
    // stop at the first matching row. Served by `voyage_photos_photo_id_idx`.
    let visible: Option<bool> = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1
                 FROM voyage_photos vp
                 JOIN voyages v ON v.id = vp.voyage_id
                WHERE vp.photo_id = $1
                  AND v.published
           )"#,
        photo_id,
    )
    .fetch_one(executor)
    .await?;

    Ok(visible.unwrap_or(false))
}

/// One placed photograph in a published voyage, for the discovery map.
///
/// Coordinates are **exact**, like every other row type here, and the
/// granularity travels with them as the stored string so that the rounding
/// still happens once at the serialization boundary rather than in this query.
/// A photograph that is in two published voyages appears twice, once per
/// voyage, because it is genuinely published twice and each voyage may round it
/// differently.
#[derive(Debug, Clone)]
pub struct DiscoverPhoto {
    pub photo_id: Uuid,
    pub voyage_id: Uuid,
    /// Stored spelling; parse with `Granularity::parse`.
    pub granularity: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Every placed photograph in a published voyage inside a rectangle.
///
/// `wraps` is for a rectangle that crosses the antimeridian, where `west` is
/// numerically greater than `east` — panning across the Pacific is not exotic,
/// and without it that pan returns either everything or nothing depending on
/// which way the comparison falls.
///
/// `limit` is a cap, not a page size: there is no cursor here because the map
/// asks for an area rather than for a sequence. The caller is expected to ask
/// for one more row than it intends to use, so that hitting the cap is
/// detectable and can be reported rather than silently changing the answer.
/// Ordering by `photo_id` makes which rows survive the cap deterministic, so
/// the same rectangle does not return a different sample each time it is asked
/// for.
pub async fn published_photos_in_bbox(
    executor: impl PgExecutor<'_>,
    south: rust_decimal::Decimal,
    north: rust_decimal::Decimal,
    west: rust_decimal::Decimal,
    east: rust_decimal::Decimal,
    wraps: bool,
    limit: i64,
) -> Result<Vec<DiscoverPhoto>, sqlx::Error> {
    // The latitude range is a range scan on `photos_location_idx`; the
    // longitude test is applied from the same index. Comparisons are against
    // `numeric` parameters rather than casting the columns to `float8`,
    // because casting a column is what makes an index on it unusable.
    //
    // `latitude!` and `longitude!` are asserted non-null. The column is
    // nullable, but `latitude IS NOT NULL` is in the predicate and the table's
    // own CHECK constraint makes latitude and longitude null together, so a row
    // that reached here has both.
    sqlx::query_as!(
        DiscoverPhoto,
        r#"SELECT p.id             AS photo_id,
                  v.id             AS voyage_id,
                  v.granularity,
                  p.latitude::float8  AS "latitude!",
                  p.longitude::float8 AS "longitude!"
             FROM photos p
             JOIN voyage_photos vp ON vp.photo_id = p.id
             JOIN voyages v        ON v.id = vp.voyage_id
            WHERE v.published
              AND p.latitude IS NOT NULL
              AND p.latitude BETWEEN $1 AND $2
              AND CASE WHEN $5 THEN (p.longitude >= $3 OR p.longitude <= $4)
                       ELSE p.longitude BETWEEN $3 AND $4
                  END
            ORDER BY p.id
            LIMIT $6"#,
        south,
        north,
        west,
        east,
        wraps,
        limit,
    )
    .fetch_all(executor)
    .await
}

#[cfg(test)]
#[path = "voyages_tests.rs"]
mod tests;
