//! Photos repository.
//!
//! CONTRACT — signatures are fixed; implementation owned by the upload agent.
//!
//! Every function takes an executor, so the callback can create a photo and
//! enqueue it inside one transaction, and tests can roll back.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

/// A photo row. Coordinates here are **exact** — granularity is applied at the
/// serialization boundary in phase 5, never in the repository, so that there is
/// exactly one place to get it right.
#[derive(Debug, Clone)]
pub struct Photo {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub content_hash: String,
    pub byte_size: i64,
    pub width: i32,
    pub height: i32,
    pub original_format: String,
    pub captured_at: Option<DateTime<Utc>>,
    pub latitude: Option<rust_decimal::Decimal>,
    pub longitude: Option<rust_decimal::Decimal>,
    pub variant_status: String,
    pub created_at: DateTime<Utc>,
}

/// What an upload knows before the row exists.
#[derive(Debug, Clone)]
pub struct NewPhoto {
    pub owner_id: Uuid,
    pub content_hash: String,
    pub byte_size: i64,
    pub width: i32,
    pub height: i32,
    pub original_format: String,
    pub captured_at: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Scale of the `latitude`/`longitude` columns. Six decimal places is about
/// 11 cm at the equator — far finer than any consumer GPS — and it is what the
/// migration declares, so rounding here rather than letting Postgres do it
/// means the value read back is the value that was bound.
const COORDINATE_SCALE: u32 = 6;

/// Converts an EXIF coordinate to the column's `numeric` type.
///
/// EXIF gives degrees as a float; the column is `numeric(9, 6)` because "the
/// number we were given" is a better thing to hold than "a number very close to
/// it" when the whole privacy model is about controlling precision (see the
/// migration).
///
/// `None` for a non-finite input, which is the only value `Decimal` cannot
/// represent. A finite but out-of-range coordinate — beyond ±90 or ±180 — is
/// *not* silently dropped here: it exceeds `numeric(9, 6)`'s three integer
/// digits and surfaces as a database error, which is the honest outcome. Range
/// checking belongs to the EXIF extractor, which knows what it read.
fn to_numeric(degrees: Option<f64>) -> Option<rust_decimal::Decimal> {
    degrees
        .and_then(rust_decimal::Decimal::from_f64_retain)
        .map(|value| value.round_dp(COORDINATE_SCALE))
}

pub async fn create(executor: impl PgExecutor<'_>, photo: &NewPhoto) -> Result<Photo, sqlx::Error> {
    // v7 rather than v4, same reasoning as `users::create`: time-ordered ids
    // land at the right edge of the primary key index, and `photos_owner_id_idx`
    // is `(owner_id, id DESC)` precisely so that ordering by the id *is* the
    // chronological ordering the library view wants.
    let id = Uuid::now_v7();

    sqlx::query_as!(
        Photo,
        r#"INSERT INTO photos
               (id, owner_id, content_hash, byte_size, width, height, original_format,
                captured_at, latitude, longitude)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, owner_id, content_hash, byte_size, width, height, original_format,
                     captured_at, latitude, longitude, variant_status, created_at"#,
        id,
        photo.owner_id,
        photo.content_hash,
        photo.byte_size,
        photo.width,
        photo.height,
        photo.original_format,
        photo.captured_at,
        to_numeric(photo.latitude),
        to_numeric(photo.longitude),
    )
    .fetch_one(executor)
    .await
}

pub async fn find_by_id(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> Result<Option<Photo>, sqlx::Error> {
    // `Ok(None)` rather than `RowNotFound`, matching `users::find_by_id`: a
    // missing photo is an ordinary outcome (a stale link, a deleted row) and
    // the handler turns it into a 404.
    sqlx::query_as!(
        Photo,
        r#"SELECT id, owner_id, content_hash, byte_size, width, height, original_format,
                  captured_at, latitude, longitude, variant_status, created_at
             FROM photos
            WHERE id = $1"#,
        id,
    )
    .fetch_optional(executor)
    .await
}

/// The uploader's library, newest first, cursor-paginated on the v7 id.
///
/// Keyset pagination rather than `OFFSET`: the id is time-ordered, so `id < $2`
/// is "older than that photo" and costs an index seek regardless of how deep the
/// page is. `OFFSET` would scan and discard every earlier row, and would also
/// skip or repeat rows when an upload lands mid-scroll.
pub async fn list_for_owner(
    executor: impl PgExecutor<'_>,
    owner_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Photo>, sqlx::Error> {
    // `$2::uuid IS NULL OR id < $2` keeps the first page and every later page
    // on one statement. The cast is required: without it Postgres cannot infer
    // a type for a parameter that only ever appears next to `IS NULL`.
    sqlx::query_as!(
        Photo,
        r#"SELECT id, owner_id, content_hash, byte_size, width, height, original_format,
                  captured_at, latitude, longitude, variant_status, created_at
             FROM photos
            WHERE owner_id = $1
              AND ($2::uuid IS NULL OR id < $2)
            ORDER BY id DESC
            LIMIT $3"#,
        owner_id,
        before,
        limit,
    )
    .fetch_all(executor)
    .await
}

pub async fn set_variant_status(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    status: &str,
) -> Result<(), sqlx::Error> {
    // The permitted spellings are enforced by the migration's CHECK constraint
    // rather than re-stated here — one definition of the state machine, in the
    // place that can actually hold the line against a bad write.
    //
    // A row that no longer exists is not an error: the worker finishes a photo
    // its owner deleted while it was being processed, and that is an ordinary
    // race, not a failure to report.
    sqlx::query!(
        "UPDATE photos SET variant_status = $2, updated_at = now() WHERE id = $1",
        id,
        status,
    )
    .execute(executor)
    .await?;

    Ok(())
}

/// Photos left `pending` by a previous run, for the worker to pick up at
/// startup. Without this a process that died mid-batch strands them forever.
pub async fn pending(executor: impl PgExecutor<'_>, limit: i64) -> Result<Vec<Uuid>, sqlx::Error> {
    // Ascending, unlike the library view: this is a drain, and the oldest
    // stranded photo is the one whose owner has been staring at a placeholder
    // the longest. Served by `photos_variant_status_idx`, which is partial on
    // exactly this predicate.
    sqlx::query_scalar!(
        "SELECT id FROM photos WHERE variant_status = 'pending' ORDER BY id ASC LIMIT $1",
        limit,
    )
    .fetch_all(executor)
    .await
}

/// Deletes a photo, returning whether its content is now unreferenced.
///
/// The caller unlinks files only when this says `true`. Deduplication means two
/// users can share one file, so an unqualified unlink deletes someone else's
/// photograph — and the count and the delete must be in the same transaction,
/// or a concurrent upload of the same bytes can slip between them.
///
/// `Ok(None)` means "no such photo, or not yours". The two are deliberately
/// indistinguishable: telling a caller that a photo exists but belongs to
/// someone else is an enumeration oracle, and the ownership predicate lives in
/// the `WHERE` clause rather than in a preceding `SELECT` so there is no window
/// between the check and the delete.
///
/// **What this does and does not guarantee.** The delete and the reference
/// check are one statement, evaluated against one snapshot, so nothing can
/// interleave between them — that is the race the doc comment above names, and
/// it is closed. What a single statement at READ COMMITTED cannot close is a
/// concurrent uploader that commits a *new* row for the same hash after this
/// statement's snapshot is taken: this reports `true`, the caller unlinks, and
/// the new photo points at a file that is gone. Closing that one needs a lock
/// shared with the upload path — `pg_advisory_xact_lock` keyed on the content
/// hash, taken by the uploader around commit-plus-insert and by the deleter
/// around this call plus the unlink. It belongs in the handler, not here,
/// because only the handler owns the transaction the lock must live in.
pub async fn delete_returning_orphaned(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner_id: Uuid,
) -> Result<Option<(String, bool)>, sqlx::Error> {
    // `AND photos.id <> $1` is load-bearing and is the thing that looks wrong
    // at a glance. A data-modifying CTE's effects are *not* visible to the rest
    // of the statement: every part reads the same snapshot, taken before the
    // statement ran, so the outer `EXISTS` still sees the row the CTE just
    // deleted. Without the exclusion this returns `false` for every photo ever
    // deleted, no file is ever unlinked, and the store grows forever — a bug
    // that no single-photo test would catch, because "did not delete the file"
    // looks exactly like "correctly kept a shared file".
    let row = sqlx::query!(
        r#"WITH deleted AS (
               DELETE FROM photos
                WHERE id = $1 AND owner_id = $2
               RETURNING content_hash
           )
           SELECT deleted.content_hash AS "content_hash!",
                  NOT EXISTS (
                      SELECT 1
                        FROM photos
                       WHERE photos.content_hash = deleted.content_hash
                         AND photos.id <> $1
                  ) AS "orphaned!"
             FROM deleted"#,
        id,
        owner_id,
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(|row| (row.content_hash, row.orphaned)))
}

#[cfg(test)]
#[path = "photos_tests.rs"]
mod tests;
