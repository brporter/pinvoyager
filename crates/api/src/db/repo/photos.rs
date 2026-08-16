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
mod tests {
    use super::*;

    use sqlx::PgPool;

    /// Distinct 64-character hex strings, so a test that means "different
    /// content" cannot accidentally collide with one that means "the same".
    fn hash_of(seed: u8) -> String {
        std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
    }

    fn new_photo(owner_id: Uuid, content_hash: &str) -> NewPhoto {
        NewPhoto {
            owner_id,
            content_hash: content_hash.to_owned(),
            byte_size: 1_234,
            width: 4032,
            height: 3024,
            original_format: "heic".to_owned(),
            captured_at: None,
            latitude: None,
            longitude: None,
        }
    }

    async fn a_user(pool: &PgPool) -> Uuid {
        crate::db::repo::users::create(pool, None, None)
            .await
            .expect("create user")
            .id
    }

    #[sqlx::test]
    async fn a_photo_round_trips(pool: PgPool) {
        let owner = a_user(&pool).await;

        let mut input = new_photo(owner, &hash_of(0x11));
        input.captured_at = Some(
            "2024-06-01T09:30:00Z"
                .parse::<DateTime<Utc>>()
                .expect("parse timestamp"),
        );
        input.latitude = Some(47.620506);
        input.longitude = Some(-122.349274);

        let created = create(&pool, &input).await.expect("create");

        assert_eq!(created.owner_id, owner);
        assert_eq!(created.content_hash, input.content_hash);
        assert_eq!(created.byte_size, 1_234);
        assert_eq!(created.width, 4032);
        assert_eq!(created.height, 3024);
        assert_eq!(created.original_format, "heic");
        assert_eq!(created.captured_at, input.captured_at);
        assert_eq!(
            created.variant_status, "pending",
            "a photo exists before its variants do"
        );

        let found = find_by_id(&pool, created.id)
            .await
            .expect("find")
            .expect("the photo exists");
        assert_eq!(found.id, created.id);
        assert_eq!(found.latitude, created.latitude);
    }

    /// D6 turns on exact coordinates surviving the round trip: granularity is
    /// applied at serialization, so anything lost here is lost for good and the
    /// later "adjust location" feature has nothing to adjust.
    #[sqlx::test]
    async fn coordinates_round_trip_at_full_stored_precision(pool: PgPool) {
        let owner = a_user(&pool).await;

        let mut input = new_photo(owner, &hash_of(0x12));
        input.latitude = Some(-33.856784);
        input.longitude = Some(151.215297);

        let created = create(&pool, &input).await.expect("create");

        assert_eq!(
            created.latitude.map(|d| d.to_string()).as_deref(),
            Some("-33.856784")
        );
        assert_eq!(
            created.longitude.map(|d| d.to_string()).as_deref(),
            Some("151.215297")
        );
    }

    /// A screenshot has no GPS. That is a normal, expected outcome — not an
    /// error and not a half-placed row.
    #[sqlx::test]
    async fn a_photo_without_gps_is_stored_unplaced(pool: PgPool) {
        let owner = a_user(&pool).await;
        let created = create(&pool, &new_photo(owner, &hash_of(0x13)))
            .await
            .expect("create");

        assert!(created.latitude.is_none());
        assert!(created.longitude.is_none());
    }

    #[sqlx::test]
    async fn an_unknown_photo_is_none_rather_than_an_error(pool: PgPool) {
        assert!(
            find_by_id(&pool, Uuid::now_v7())
                .await
                .expect("find")
                .is_none()
        );
    }

    #[sqlx::test]
    async fn the_library_lists_newest_first_and_only_the_owner(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;

        let first = create(&pool, &new_photo(owner, &hash_of(0x21)))
            .await
            .expect("create");
        let second = create(&pool, &new_photo(owner, &hash_of(0x22)))
            .await
            .expect("create");
        let third = create(&pool, &new_photo(owner, &hash_of(0x23)))
            .await
            .expect("create");
        create(&pool, &new_photo(stranger, &hash_of(0x24)))
            .await
            .expect("create");

        let page = list_for_owner(&pool, owner, None, 10).await.expect("list");

        assert_eq!(
            page.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![third.id, second.id, first.id],
            "newest first, and nobody else's photos"
        );
    }

    /// The cursor is exclusive and the pages must tile — no row seen twice, no
    /// row skipped.
    #[sqlx::test]
    async fn the_cursor_walks_the_library_without_gaps_or_repeats(pool: PgPool) {
        let owner = a_user(&pool).await;

        let mut ids = Vec::new();
        for seed in 0x30..0x35 {
            ids.push(
                create(&pool, &new_photo(owner, &hash_of(seed)))
                    .await
                    .expect("create")
                    .id,
            );
        }
        ids.reverse();

        let mut walked = Vec::new();
        let mut cursor = None;
        loop {
            let page = list_for_owner(&pool, owner, cursor, 2).await.expect("list");
            if page.is_empty() {
                break;
            }
            cursor = page.last().map(|p| p.id);
            walked.extend(page.into_iter().map(|p| p.id));
        }

        assert_eq!(walked, ids);
    }

    #[sqlx::test]
    async fn pending_returns_the_worker_queue_oldest_first(pool: PgPool) {
        let owner = a_user(&pool).await;

        let first = create(&pool, &new_photo(owner, &hash_of(0x41)))
            .await
            .expect("create");
        let second = create(&pool, &new_photo(owner, &hash_of(0x42)))
            .await
            .expect("create");

        assert_eq!(
            pending(&pool, 10).await.expect("pending"),
            vec![first.id, second.id]
        );

        set_variant_status(&pool, first.id, "ready")
            .await
            .expect("set status");

        assert_eq!(
            pending(&pool, 10).await.expect("pending"),
            vec![second.id],
            "a finished photo leaves the queue"
        );
        assert_eq!(
            find_by_id(&pool, first.id)
                .await
                .expect("find")
                .expect("exists")
                .variant_status,
            "ready"
        );
    }

    /// The worker can finish a photo its owner deleted mid-processing. That is
    /// a race, not a failure.
    #[sqlx::test]
    async fn setting_the_status_of_a_vanished_photo_is_not_an_error(pool: PgPool) {
        set_variant_status(&pool, Uuid::now_v7(), "ready")
            .await
            .expect("no row is not an error");
    }

    /// The plain case: the only photo referencing this content goes away, so
    /// the files may be unlinked.
    #[sqlx::test]
    async fn deleting_the_last_reference_reports_the_content_orphaned(pool: PgPool) {
        let owner = a_user(&pool).await;
        let hash = hash_of(0x51);
        let photo = create(&pool, &new_photo(owner, &hash))
            .await
            .expect("create");

        let outcome = delete_returning_orphaned(&pool, photo.id, owner)
            .await
            .expect("delete")
            .expect("the photo existed");

        assert_eq!(outcome, (hash, true));
        assert!(find_by_id(&pool, photo.id).await.expect("find").is_none());
    }

    /// T11, and the reason this function exists at all: two users uploaded the
    /// same photograph and share one file on disk. Deleting one must not report
    /// the content orphaned, or the survivor's image disappears.
    #[sqlx::test]
    async fn deleting_one_of_two_photos_sharing_content_keeps_the_file(pool: PgPool) {
        let alice = a_user(&pool).await;
        let bob = a_user(&pool).await;
        let hash = hash_of(0x52);

        let hers = create(&pool, &new_photo(alice, &hash))
            .await
            .expect("create");
        let his = create(&pool, &new_photo(bob, &hash)).await.expect("create");

        let first = delete_returning_orphaned(&pool, hers.id, alice)
            .await
            .expect("delete")
            .expect("the photo existed");
        assert_eq!(
            first,
            (hash.clone(), false),
            "Bob's photo still references this content"
        );

        // ...and once Bob deletes his, the content really is unreferenced.
        let second = delete_returning_orphaned(&pool, his.id, bob)
            .await
            .expect("delete")
            .expect("the photo existed");
        assert_eq!(second, (hash, true));
    }

    /// The exclusion of the deleted row itself. If the outer `EXISTS` counted
    /// the row the CTE removed — which is what a naive reading of the statement
    /// gives, because the snapshot still contains it — this would report
    /// `false` and no file would ever be unlinked.
    #[sqlx::test]
    async fn a_photo_never_counts_as_its_own_surviving_reference(pool: PgPool) {
        let owner = a_user(&pool).await;
        let hash = hash_of(0x53);

        // Two photos, same owner, same content: the self-reference case is the
        // easiest one to get wrong when both rows look alike.
        let first = create(&pool, &new_photo(owner, &hash))
            .await
            .expect("create");
        let second = create(&pool, &new_photo(owner, &hash))
            .await
            .expect("create");

        assert!(
            !delete_returning_orphaned(&pool, first.id, owner)
                .await
                .expect("delete")
                .expect("existed")
                .1
        );
        assert!(
            delete_returning_orphaned(&pool, second.id, owner)
                .await
                .expect("delete")
                .expect("existed")
                .1
        );
    }

    /// Owner-scoped in the `WHERE` clause. A stranger's delete must be a no-op
    /// on the row *and* must not report the content orphaned, or one user could
    /// unlink another's file without ever owning a photo.
    #[sqlx::test]
    async fn a_stranger_cannot_delete_and_learns_nothing(pool: PgPool) {
        let owner = a_user(&pool).await;
        let stranger = a_user(&pool).await;
        let photo = create(&pool, &new_photo(owner, &hash_of(0x54)))
            .await
            .expect("create");

        assert!(
            delete_returning_orphaned(&pool, photo.id, stranger)
                .await
                .expect("delete")
                .is_none(),
            "not yours is indistinguishable from does not exist"
        );
        assert!(
            find_by_id(&pool, photo.id).await.expect("find").is_some(),
            "the row survives"
        );
    }

    #[sqlx::test]
    async fn deleting_a_photo_that_does_not_exist_is_none(pool: PgPool) {
        let owner = a_user(&pool).await;
        assert!(
            delete_returning_orphaned(&pool, Uuid::now_v7(), owner)
                .await
                .expect("delete")
                .is_none()
        );
    }

    /// Two concurrent deletes of the last two rows sharing a hash. Exactly one
    /// row is removed by each, and neither may claim the content is orphaned
    /// while the other's row is still visible to it — the outcome that would
    /// leave a live photo pointing at an unlinked file. Leaking the file
    /// instead is the safe direction to fail in, and it is what happens here.
    #[sqlx::test]
    async fn concurrent_deletes_never_both_claim_the_last_reference(pool: PgPool) {
        let alice = a_user(&pool).await;
        let bob = a_user(&pool).await;
        let hash = hash_of(0x55);

        let hers = create(&pool, &new_photo(alice, &hash))
            .await
            .expect("create");
        let his = create(&pool, &new_photo(bob, &hash)).await.expect("create");

        let a = pool.clone();
        let b = pool.clone();
        let (first, second) = tokio::join!(
            tokio::spawn(async move { delete_returning_orphaned(&a, hers.id, alice).await }),
            tokio::spawn(async move { delete_returning_orphaned(&b, his.id, bob).await }),
        );

        let claims = [first, second]
            .into_iter()
            .filter(|result| {
                matches!(
                    result.as_ref().expect("task did not panic"),
                    Ok(Some((_, true)))
                )
            })
            .count();

        assert!(
            claims <= 1,
            "two deleters both claiming the last reference would unlink a file \
             twice; only the second one to run may claim it"
        );
    }
}
