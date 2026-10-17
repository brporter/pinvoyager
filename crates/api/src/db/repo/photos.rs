//! Photo rows and async queries. Executor parameters let callers use a pool
//! or combine operations in a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

/// Photo row with exact stored coordinates. Public response construction
/// applies the voyage's granularity; owner metadata retains these values.
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
    pub voyage_id: Uuid,
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

/// Decimal places stored for latitude and longitude. Round before binding
/// so the bound value matches the column's six-decimal scale.
const COORDINATE_SCALE: u32 = 6;

/// Converts an optional coordinate to a six-decimal `Decimal`.
///
/// Returns `None` for absent or unrepresentable input. Does not check geographic
/// bounds; upload EXIF validation checks latitude and longitude before storage.
fn to_numeric(degrees: Option<f64>) -> Option<rust_decimal::Decimal> {
    degrees
        .and_then(rust_decimal::Decimal::from_f64_retain)
        .map(|value| value.round_dp(COORDINATE_SCALE))
}

/// Serialize storage writes and reference-counted deletion for identical bytes.
pub async fn lock_content(
    connection: &mut sqlx::PgConnection,
    hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(hash)
        .execute(connection)
        .await?;
    Ok(())
}

pub async fn create(executor: impl PgExecutor<'_>, photo: &NewPhoto) -> Result<Photo, sqlx::Error> {
    // v7 rather than v4, same reasoning as `users::create`: time-ordered ids
    // land at the right edge of the primary key index, and `photos_owner_id_idx`
    // is `(owner_id, id DESC)` precisely so that ordering by the id *is* the
    // chronological ordering an owner metadata query wants.
    let id = Uuid::now_v7();

    sqlx::query_as!(
        Photo,
        r#"INSERT INTO photos
               (id, owner_id, content_hash, byte_size, width, height, original_format,
                captured_at, latitude, longitude, voyage_id, position)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                   COALESCE((SELECT max(position) FROM photos WHERE voyage_id = $11), 0) + 1)
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
        photo.voyage_id,
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

/// The uploader's photos, newest first, cursor-paginated on the v7 id.
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

/// Owner-only metadata for one voyage, newest first.
pub async fn list_for_voyage(
    executor: impl PgExecutor<'_>,
    owner_id: Uuid,
    voyage_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Photo>, sqlx::Error> {
    sqlx::query_as!(Photo,
        r#"SELECT p.id, p.owner_id, p.content_hash, p.byte_size, p.width, p.height, p.original_format,
                  p.captured_at, p.latitude, p.longitude, p.variant_status, p.created_at
             FROM photos p JOIN voyage_photos vp ON vp.photo_id = p.id
            WHERE p.owner_id = $1 AND vp.voyage_id = $2
              AND ($3::uuid IS NULL OR p.id < $3)
            ORDER BY p.id DESC LIMIT $4"#,
        owner_id, voyage_id, before, limit,
    ).fetch_all(executor).await
}

pub async fn set_variant_status(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    status: &str,
) -> Result<(), sqlx::Error> {
    // The database rejects unsupported status strings. Updating a deleted row
    // succeeds without changing anything.
    sqlx::query!(
        "UPDATE photos SET variant_status = $2, updated_at = now() WHERE id = $1",
        id,
        status,
    )
    .execute(executor)
    .await?;

    Ok(())
}

/// Returns a page of pending photos for recovery, ordered by ascending ID.
pub async fn pending(executor: impl PgExecutor<'_>, limit: i64) -> Result<Vec<Uuid>, sqlx::Error> {
    // Ascending, unlike metadata pagination: this is a drain, and the oldest
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

/// Deletes an owned photo and reports whether its content has other references.
///
/// Returns `Ok(None)` for a missing or foreign photo and propagates SQL errors.
/// The returned reference check alone does not authorize file removal: the caller
/// must schedule cleanup in the deletion transaction. Cleanup locks the content
/// and checks references again after commit before removing files.
pub async fn delete_returning_orphaned(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner_id: Uuid,
) -> Result<Option<(String, bool)>, sqlx::Error> {
    // A modifying CTE and its outer SELECT share the pre-delete snapshot.
    // Exclude the deleted ID explicitly when checking remaining references.
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
