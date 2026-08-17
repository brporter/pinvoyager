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
/// alternative silently half-honours a request the caller believes was applied
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
    // The signature has no way to report "not yours", so this is defence in
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
mod tests {
    use super::*;

    use sqlx::PgPool;

    use crate::db::repo::photos::{self, NewPhoto};

    fn hash_of(seed: u8) -> String {
        std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
    }

    fn at(timestamp: &str) -> DateTime<Utc> {
        timestamp.parse().expect("parse timestamp")
    }

    async fn a_user(pool: &PgPool) -> Uuid {
        crate::db::repo::users::create(pool, None, None)
            .await
            .expect("create user")
            .id
    }

    async fn a_photo(pool: &PgPool, owner_id: Uuid, seed: u8) -> Uuid {
        a_photo_captured(pool, owner_id, seed, None).await
    }

    async fn a_photo_captured(
        pool: &PgPool,
        owner_id: Uuid,
        seed: u8,
        captured_at: Option<DateTime<Utc>>,
    ) -> Uuid {
        photos::create(
            pool,
            &NewPhoto {
                owner_id,
                content_hash: hash_of(seed),
                byte_size: 1_234,
                width: 4032,
                height: 3024,
                original_format: "heic".to_owned(),
                captured_at,
                latitude: None,
                longitude: None,
            },
        )
        .await
        .expect("create photo")
        .id
    }

    fn new_voyage(owner_id: Uuid) -> NewVoyage {
        NewVoyage {
            owner_id,
            title: "Hokkaido".to_owned(),
            description: Some("Sapporo to Otaru".to_owned()),
            granularity: "exact".to_owned(),
        }
    }

    async fn a_voyage(pool: &PgPool, owner_id: Uuid) -> Voyage {
        create(pool, &new_voyage(owner_id)).await.expect("create")
    }

    /// Publishing through the real code path, so `published_at` is whatever
    /// `update` decided rather than something the test invented.
    async fn publish(pool: &PgPool, voyage: &Voyage) -> Voyage {
        update(
            pool,
            voyage.id,
            voyage.owner_id,
            &VoyagePatch {
                published: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("publish")
        .expect("the voyage is the owner's")
    }

    async fn order_of(pool: &PgPool, voyage_id: Uuid) -> Vec<Uuid> {
        photos_for_voyage(pool, voyage_id)
            .await
            .expect("photos")
            .into_iter()
            .map(|p| p.photo_id)
            .collect()
    }

    #[sqlx::test]
    async fn a_voyage_is_created_as_a_draft(pool: PgPool) {
        let owner = a_user(&pool).await;
        let created = create(&pool, &new_voyage(owner)).await.expect("create");

        assert_eq!(created.owner_id, owner);
        assert_eq!(created.title, "Hokkaido");
        assert_eq!(created.description.as_deref(), Some("Sapporo to Otaru"));
        assert_eq!(created.granularity, "exact");
        assert!(
            !created.published,
            "T8: a half-populated voyage must not be world-readable the moment \
             it exists"
        );
        assert!(created.published_at.is_none());

        let found = find_by_id(&pool, created.id)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(found.id, created.id);
        assert_eq!(found.title, created.title);
    }

    #[sqlx::test]
    async fn an_unknown_voyage_is_none_rather_than_an_error(pool: PgPool) {
        assert!(
            find_by_id(&pool, Uuid::now_v7())
                .await
                .expect("find")
                .is_none()
        );
    }

    #[sqlx::test]
    async fn a_granularity_outside_the_permitted_set_is_refused(pool: PgPool) {
        let owner = a_user(&pool).await;
        let mut input = new_voyage(owner);
        input.granularity = "street".to_owned();

        assert!(
            create(&pool, &input).await.is_err(),
            "the migration's CHECK is the definition of the permitted set"
        );
    }

    /// The reason `description` is `Option<Option<String>>`. A patch that does
    /// not mention it must leave it alone; a patch that sets it to `None` must
    /// clear it. A `COALESCE` over a single parameter cannot do both.
    #[sqlx::test]
    async fn a_patch_distinguishes_clearing_a_description_from_ignoring_it(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let untouched = update(
            &pool,
            voyage.id,
            owner,
            &VoyagePatch {
                title: Some("Hokkaido, again".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("update")
        .expect("exists");
        assert_eq!(untouched.title, "Hokkaido, again");
        assert_eq!(
            untouched.description.as_deref(),
            Some("Sapporo to Otaru"),
            "an absent field is not a null field"
        );

        let rewritten = update(
            &pool,
            voyage.id,
            owner,
            &VoyagePatch {
                description: Some(Some("Otaru to Furano".to_owned())),
                ..Default::default()
            },
        )
        .await
        .expect("update")
        .expect("exists");
        assert_eq!(rewritten.description.as_deref(), Some("Otaru to Furano"));

        let cleared = update(
            &pool,
            voyage.id,
            owner,
            &VoyagePatch {
                description: Some(None),
                ..Default::default()
            },
        )
        .await
        .expect("update")
        .expect("exists");
        assert!(
            cleared.description.is_none(),
            "Some(None) is an explicit request to clear the description"
        );
        assert_eq!(cleared.title, "Hokkaido, again", "and nothing else moved");
    }

    /// The discovery feed orders by `published_at`, so republishing must not
    /// jump the queue. The timestamp is written once, on the first transition,
    /// and never again.
    #[sqlx::test]
    async fn publishing_stamps_the_time_once_and_never_restamps_it(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let published = publish(&pool, &voyage).await;
        assert!(published.published);
        let first_stamp = published.published_at.expect("stamped on first publish");

        let retitled = update(
            &pool,
            voyage.id,
            owner,
            &VoyagePatch {
                title: Some("Renamed".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("update")
        .expect("exists");
        assert_eq!(
            retitled.published_at,
            Some(first_stamp),
            "an unrelated edit does not restamp"
        );

        let unpublished = update(
            &pool,
            voyage.id,
            owner,
            &VoyagePatch {
                published: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update")
        .expect("exists");
        assert!(!unpublished.published);
        assert_eq!(
            unpublished.published_at,
            Some(first_stamp),
            "unpublishing hides the voyage; it does not erase when it first \
             became visible"
        );

        let republished = publish(&pool, &voyage).await;
        assert_eq!(
            republished.published_at,
            Some(first_stamp),
            "republishing must not move the voyage to the front of the feed"
        );
    }

    #[sqlx::test]
    async fn a_stranger_cannot_update_and_learns_nothing(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        assert!(
            update(
                &pool,
                voyage.id,
                stranger,
                &VoyagePatch {
                    title: Some("Hijacked".to_owned()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .is_none(),
            "not yours is indistinguishable from does not exist"
        );

        assert_eq!(
            find_by_id(&pool, voyage.id)
                .await
                .expect("find")
                .expect("exists")
                .title,
            "Hokkaido",
            "and the row is untouched"
        );

        assert!(
            update(&pool, Uuid::now_v7(), owner, &VoyagePatch::default())
                .await
                .expect("update")
                .is_none()
        );
    }

    /// The migration's stated promise: the join rows cascade, the photographs
    /// do not. Someone tidying up a draft is not asking to destroy the images.
    #[sqlx::test]
    async fn deleting_a_voyage_returns_its_photos_to_the_library(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;
        let first = a_photo(&pool, owner, 0x11).await;
        let second = a_photo(&pool, owner, 0x12).await;

        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[first, second])
                .await
                .expect("attach"),
            2
        );

        assert!(delete(&pool, voyage.id, owner).await.expect("delete"));

        assert!(find_by_id(&pool, voyage.id).await.expect("find").is_none());
        assert!(
            photos_for_voyage(&pool, voyage.id)
                .await
                .expect("photos")
                .is_empty(),
            "the join rows cascade"
        );
        for photo in [first, second] {
            assert!(
                photos::find_by_id(&pool, photo)
                    .await
                    .expect("find photo")
                    .is_some(),
                "the photograph itself survives"
            );
        }
    }

    #[sqlx::test]
    async fn a_stranger_cannot_delete(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        assert!(!delete(&pool, voyage.id, stranger).await.expect("delete"));
        assert!(find_by_id(&pool, voyage.id).await.expect("find").is_some());
        assert!(
            !delete(&pool, Uuid::now_v7(), owner).await.expect("delete"),
            "deleting nothing is false, not an error"
        );
    }

    /// T3, at the repository level: the foreign photo is not attached, and the
    /// count tells the handler that the request was not honoured in full so it
    /// can roll back and return 403.
    #[sqlx::test]
    async fn attaching_another_users_photo_attaches_nothing(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;
        let theirs = a_photo(&pool, stranger, 0x21).await;

        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[theirs])
                .await
                .expect("attach"),
            0
        );
        assert!(
            photos_for_voyage(&pool, voyage.id)
                .await
                .expect("photos")
                .is_empty()
        );

        // Mixed in with the caller's own photo, the foreign one is still
        // dropped and the shortfall in the count is what the handler sees.
        let mine = a_photo(&pool, owner, 0x22).await;
        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[mine, theirs])
                .await
                .expect("attach"),
            1
        );
        assert_eq!(order_of(&pool, voyage.id).await, vec![mine]);
    }

    #[sqlx::test]
    async fn attaching_to_someone_elses_voyage_attaches_nothing(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;
        let theirs = a_photo(&pool, stranger, 0x23).await;

        assert_eq!(
            attach_photos(&pool, voyage.id, stranger, &[theirs])
                .await
                .expect("attach"),
            0,
            "owning the photograph is not enough; the voyage must be yours too"
        );
        assert!(
            photos_for_voyage(&pool, voyage.id)
                .await
                .expect("photos")
                .is_empty()
        );
    }

    /// The race the doc comment is about. A check-then-insert would have a
    /// window between the `SELECT` that says "not yours" and the `INSERT`;
    /// with the predicate inside the write there is no window, so hammering it
    /// concurrently cannot produce a single row.
    #[sqlx::test]
    async fn the_ownership_predicate_holds_under_concurrency(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;
        let mine = a_photo(&pool, owner, 0x24).await;

        // The owner attaches their own photo while the stranger repeatedly
        // tries to attach the same photo to the same voyage. Every interleaving
        // of the two is exercised over enough rounds; none of them may leave
        // the stranger having written anything.
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let a = pool.clone();
            let b = pool.clone();
            let voyage_id = voyage.id;
            tasks.push(tokio::spawn(async move {
                let wanted = [mine];
                let (owners, strangers) = tokio::join!(
                    attach_photos(&a, voyage_id, owner, &wanted),
                    attach_photos(&b, voyage_id, stranger, &wanted),
                );
                (owners.expect("owner attach"), strangers.expect("stranger"))
            }));
        }

        let mut owner_inserts = 0;
        for task in tasks {
            let (owners, strangers) = task.await.expect("task did not panic");
            owner_inserts += owners;
            assert_eq!(strangers, 0, "a stranger never inserts, however it races");
        }

        assert_eq!(
            owner_inserts, 1,
            "the primary key makes the owner's own repeats idempotent too"
        );
        assert_eq!(order_of(&pool, voyage.id).await, vec![mine]);
    }

    /// T4: position defaults to capture time. A batch is ordered among itself
    /// rather than by the order of the caller's array, and photos with no EXIF
    /// timestamp go to the back instead of the front.
    #[sqlx::test]
    async fn a_batch_is_positioned_by_capture_time_with_undated_photos_last(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let june = a_photo_captured(&pool, owner, 0x31, Some(at("2024-06-01T09:30:00Z"))).await;
        let april = a_photo_captured(&pool, owner, 0x32, Some(at("2024-04-02T11:00:00Z"))).await;
        let screenshot = a_photo_captured(&pool, owner, 0x33, None).await;
        let scan = a_photo_captured(&pool, owner, 0x34, None).await;

        // Deliberately shuffled relative to capture time.
        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[screenshot, june, scan, april])
                .await
                .expect("attach"),
            4
        );

        assert_eq!(
            order_of(&pool, voyage.id).await,
            vec![april, june, screenshot, scan],
            "dated photos in capture order, then undated ones in upload order"
        );
    }

    #[sqlx::test]
    async fn a_later_attach_appends_after_what_is_already_there(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        // The newcomer was captured *earlier* than the photos already in the
        // voyage. It still lands at the end: capture time orders a batch among
        // itself, it does not reshuffle an order the owner may have set.
        let first = a_photo_captured(&pool, owner, 0x41, Some(at("2024-06-01T00:00:00Z"))).await;
        let second = a_photo_captured(&pool, owner, 0x42, Some(at("2024-06-02T00:00:00Z"))).await;
        let older = a_photo_captured(&pool, owner, 0x43, Some(at("2020-01-01T00:00:00Z"))).await;

        attach_photos(&pool, voyage.id, owner, &[first, second])
            .await
            .expect("attach");
        attach_photos(&pool, voyage.id, owner, &[older])
            .await
            .expect("attach");

        assert_eq!(order_of(&pool, voyage.id).await, vec![first, second, older]);
    }

    #[sqlx::test]
    async fn re_attaching_a_photo_already_present_adds_nothing(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;
        let photo = a_photo(&pool, owner, 0x44).await;

        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[photo])
                .await
                .expect("attach"),
            1
        );
        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[photo])
                .await
                .expect("attach"),
            0
        );
        assert_eq!(order_of(&pool, voyage.id).await, vec![photo]);
    }

    #[sqlx::test]
    async fn attaching_nothing_is_not_an_error(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        assert_eq!(
            attach_photos(&pool, voyage.id, owner, &[])
                .await
                .expect("attach"),
            0
        );
    }

    #[sqlx::test]
    async fn detaching_is_scoped_to_the_voyages_owner(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;
        let photo = a_photo(&pool, owner, 0x51).await;

        attach_photos(&pool, voyage.id, owner, &[photo])
            .await
            .expect("attach");

        assert!(
            !detach_photo(&pool, voyage.id, stranger, photo)
                .await
                .expect("detach"),
            "a stranger removes nothing"
        );
        assert_eq!(order_of(&pool, voyage.id).await, vec![photo]);

        assert!(
            detach_photo(&pool, voyage.id, owner, photo)
                .await
                .expect("detach")
        );
        assert!(order_of(&pool, voyage.id).await.is_empty());

        assert!(
            !detach_photo(&pool, voyage.id, owner, photo)
                .await
                .expect("detach"),
            "detaching what is not there is false, not an error"
        );
        assert!(
            photos::find_by_id(&pool, photo)
                .await
                .expect("find photo")
                .is_some(),
            "detaching returns the photo to the library, it does not delete it"
        );
    }

    #[sqlx::test]
    async fn reorder_applies_the_requested_sequence(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let mut photos_in = Vec::new();
        for seed in 0x61..0x65 {
            photos_in.push(a_photo(&pool, owner, seed).await);
        }
        attach_photos(&pool, voyage.id, owner, &photos_in)
            .await
            .expect("attach");

        let mut reversed = photos_in.clone();
        reversed.reverse();
        reorder(&pool, voyage.id, owner, &reversed)
            .await
            .expect("reorder");

        assert_eq!(order_of(&pool, voyage.id).await, reversed);

        let positions: Vec<i32> = photos_for_voyage(&pool, voyage.id)
            .await
            .expect("photos")
            .into_iter()
            .map(|p| p.position)
            .collect();
        assert_eq!(
            positions,
            vec![1, 2, 3, 4],
            "the sequence is renumbered contiguously from 1"
        );
    }

    /// The documented behaviour for a stale list: ids that are not in the
    /// voyage are ignored, and photos the caller left out keep their relative
    /// order behind the ones that were named. Neither is an error, because both
    /// are what an ordinary race between two tabs produces.
    #[sqlx::test]
    async fn reorder_ignores_foreign_ids_and_appends_omitted_photos(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let a = a_photo(&pool, owner, 0x71).await;
        let b = a_photo(&pool, owner, 0x72).await;
        let c = a_photo(&pool, owner, 0x73).await;
        attach_photos(&pool, voyage.id, owner, &[a, b, c])
            .await
            .expect("attach");
        assert_eq!(order_of(&pool, voyage.id).await, vec![a, b, c]);

        let theirs = a_photo(&pool, stranger, 0x74).await;
        reorder(
            &pool,
            voyage.id,
            owner,
            &[c, theirs, Uuid::now_v7(), c, /* a duplicate */ c],
        )
        .await
        .expect("reorder");

        assert_eq!(
            order_of(&pool, voyage.id).await,
            vec![c, a, b],
            "the named photo leads; the omitted ones keep their relative order \
             behind it; a foreign id, an unknown id and a repeat are all no-ops"
        );
    }

    #[sqlx::test]
    async fn a_stranger_cannot_reorder(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let a = a_photo(&pool, owner, 0x81).await;
        let b = a_photo(&pool, owner, 0x82).await;
        attach_photos(&pool, voyage.id, owner, &[a, b])
            .await
            .expect("attach");

        reorder(&pool, voyage.id, stranger, &[b, a])
            .await
            .expect("reorder");

        assert_eq!(
            order_of(&pool, voyage.id).await,
            vec![a, b],
            "the ownership predicate is inside the statement"
        );
    }

    /// Atomicity. Two reorders racing must leave one of the two requested
    /// sequences, never a splice of both — which is what a row-at-a-time
    /// implementation would produce, silently, since `position` has no unique
    /// constraint to trip over.
    #[sqlx::test]
    async fn concurrent_reorders_never_interleave(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let mut photos_in = Vec::new();
        for seed in 0x91..0x97 {
            photos_in.push(a_photo(&pool, owner, seed).await);
        }
        attach_photos(&pool, voyage.id, owner, &photos_in)
            .await
            .expect("attach");

        let one: Vec<Uuid> = photos_in.iter().rev().copied().collect();
        let two: Vec<Uuid> = photos_in
            .iter()
            .skip(3)
            .chain(photos_in.iter().take(3))
            .copied()
            .collect();

        for _ in 0..8 {
            let a = pool.clone();
            let b = pool.clone();
            let (first, second) = (one.clone(), two.clone());
            let voyage_id = voyage.id;
            let (left, right) = tokio::join!(
                tokio::spawn(async move { reorder(&a, voyage_id, owner, &first).await }),
                tokio::spawn(async move { reorder(&b, voyage_id, owner, &second).await }),
            );
            left.expect("task").expect("reorder");
            right.expect("task").expect("reorder");

            let observed = order_of(&pool, voyage.id).await;
            assert!(
                observed == one || observed == two,
                "expected one of the two requested sequences, got {observed:?}"
            );
        }
    }

    /// T6/T9. A loop of per-photo lookups reads `photos` once per row; the
    /// joined statement reads it once. The counter is per-transaction, so the
    /// measurement is exact and nothing another test does can perturb it.
    ///
    /// `enable_nestloop = off` is a test artifact, not a claim about the plan.
    /// A nested loop is a perfectly good plan for a single statement, but it
    /// rescans the inner relation once per outer row and each rescan is counted
    /// — which is indistinguishable from forty-eight separate statements in
    /// this counter. Disabling it removes that ambiguity without weakening the
    /// assertion: forty-eight round trips would still record forty-eight scans
    /// whatever the join strategy, because each one is its own plan.
    #[sqlx::test]
    async fn loading_a_voyage_of_48_photos_reads_the_photos_table_once(pool: PgPool) {
        const PHOTO_COUNT: usize = 48;

        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let mut photos_in = Vec::new();
        for seed in 0..PHOTO_COUNT {
            photos_in.push(a_photo(&pool, owner, seed as u8).await);
        }
        attach_photos(&pool, voyage.id, owner, &photos_in)
            .await
            .expect("attach");

        let mut tx = pool.begin().await.expect("begin");
        sqlx::query("SET LOCAL enable_nestloop = off")
            .execute(&mut *tx)
            .await
            .expect("set");

        async fn scans_of_photos(tx: &mut sqlx::PgConnection) -> i64 {
            sqlx::query_scalar!(
                // `sum()` over bigint yields numeric, hence the cast back.
                r#"SELECT COALESCE(
                              sum(COALESCE(seq_scan, 0) + COALESCE(idx_scan, 0)), 0
                          )::bigint AS "scans!"
                     FROM pg_stat_xact_user_tables
                    WHERE relname = 'photos'"#,
            )
            .fetch_one(tx)
            .await
            .expect("read scan counter")
        }

        let before = scans_of_photos(&mut tx).await;
        let loaded = photos_for_voyage(&mut *tx, voyage.id)
            .await
            .expect("photos");
        let after = scans_of_photos(&mut tx).await;
        tx.rollback().await.expect("rollback");

        assert_eq!(loaded.len(), PHOTO_COUNT);
        assert_eq!(
            after - before,
            1,
            "one scan of photos for the whole voyage; {PHOTO_COUNT} would mean \
             an await in a loop"
        );
    }

    /// Exact coordinates reach the caller unrounded. This file is not the
    /// granularity boundary and must not behave like one — `http::voyages` is.
    #[sqlx::test]
    async fn photos_for_voyage_returns_exact_coordinates(pool: PgPool) {
        let owner = a_user(&pool).await;
        let voyage = a_voyage(&pool, owner).await;

        let placed = photos::create(
            &pool,
            &NewPhoto {
                owner_id: owner,
                content_hash: hash_of(0xa1),
                byte_size: 1,
                width: 4032,
                height: 3024,
                original_format: "jpeg".to_owned(),
                captured_at: None,
                latitude: Some(43.061936),
                longitude: Some(141.354292),
            },
        )
        .await
        .expect("create photo")
        .id;
        let unplaced = a_photo(&pool, owner, 0xa2).await;

        attach_photos(&pool, voyage.id, owner, &[placed, unplaced])
            .await
            .expect("attach");

        let loaded = photos_for_voyage(&pool, voyage.id).await.expect("photos");
        let placed_row = loaded
            .iter()
            .find(|p| p.photo_id == placed)
            .expect("the placed photo");

        assert_eq!(placed_row.latitude, Some(43.061936));
        assert_eq!(placed_row.longitude, Some(141.354292));
        assert_eq!(placed_row.width, 4032);
        assert_eq!(placed_row.height, 3024);
        assert_eq!(placed_row.variant_status, "pending");

        let unplaced_row = loaded
            .iter()
            .find(|p| p.photo_id == unplaced)
            .expect("the unplaced photo");
        assert!(
            unplaced_row.latitude.is_none(),
            "a photo with no GPS is unplaced, not an error"
        );
    }

    #[sqlx::test]
    async fn the_feed_lists_published_voyages_newest_first(pool: PgPool) {
        let owner = a_user(&pool).await;

        let first = a_voyage(&pool, owner).await;
        let second = a_voyage(&pool, owner).await;
        let draft = a_voyage(&pool, owner).await;

        publish(&pool, &first).await;
        publish(&pool, &second).await;

        let feed = list_published(&pool, None, 10).await.expect("list");

        assert_eq!(
            feed.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![second.id, first.id],
            "most recently published first"
        );
        assert!(
            !feed.iter().any(|v| v.id == draft.id),
            "a draft is not in the discovery feed"
        );
    }

    /// T7. The cursor is a keyset on `published_at`, so a voyage published
    /// during a traversal is newer than every cursor already issued and cannot
    /// displace a row the reader has not reached yet. With `OFFSET` the reader
    /// would silently skip one row per remaining page.
    #[sqlx::test]
    async fn the_feed_cursor_is_stable_across_a_publication_mid_traversal(pool: PgPool) {
        let owner = a_user(&pool).await;

        let mut expected = Vec::new();
        for _ in 0..5 {
            let voyage = a_voyage(&pool, owner).await;
            expected.push(publish(&pool, &voyage).await.id);
        }
        expected.reverse();

        let mut walked = Vec::new();
        let mut cursor = None;
        let mut interrupted = false;
        loop {
            let page = list_published(&pool, cursor, 2).await.expect("list");
            if page.is_empty() {
                break;
            }
            cursor = page.last().and_then(|v| v.published_at);
            walked.extend(page.into_iter().map(|v| v.id));

            if !interrupted {
                // A new voyage becomes visible after the first page has been
                // handed out.
                let latecomer = a_voyage(&pool, owner).await;
                publish(&pool, &latecomer).await;
                interrupted = true;
            }
        }

        assert!(interrupted, "the traversal must have been interrupted");
        assert_eq!(
            walked.len(),
            walked
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "no row is seen twice"
        );
        assert_eq!(
            walked.iter().take(5).copied().collect::<Vec<_>>(),
            expected,
            "every pre-existing voyage is seen exactly once, in order"
        );
    }

    #[sqlx::test]
    async fn a_users_own_list_includes_drafts_and_only_their_own(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;

        let first = a_voyage(&pool, owner).await;
        let second = a_voyage(&pool, owner).await;
        let third = a_voyage(&pool, owner).await;
        let theirs = a_voyage(&pool, stranger).await;
        publish(&pool, &second).await;

        let page = list_for_owner(&pool, owner, None, 10).await.expect("list");

        assert_eq!(
            page.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![third.id, second.id, first.id],
            "newest first, drafts included"
        );
        assert!(!page.iter().any(|v| v.id == theirs.id));
    }

    #[sqlx::test]
    async fn the_owner_cursor_walks_without_gaps_or_repeats(pool: PgPool) {
        let owner = a_user(&pool).await;

        let mut ids = Vec::new();
        for _ in 0..5 {
            ids.push(a_voyage(&pool, owner).await.id);
        }
        ids.reverse();

        let mut walked = Vec::new();
        let mut cursor = None;
        loop {
            let page = list_for_owner(&pool, owner, cursor, 2).await.expect("list");
            if page.is_empty() {
                break;
            }
            cursor = page.last().map(|v| v.id);
            walked.extend(page.into_iter().map(|v| v.id));
        }

        assert_eq!(walked, ids);
    }
}
