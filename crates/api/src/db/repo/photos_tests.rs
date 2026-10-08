use super::*;

use sqlx::PgPool;

/// Distinct 64-character hex strings, so a test that means "different
/// content" cannot accidentally collide with one that means "the same".
fn hash_of(seed: u8) -> String {
    std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
}

fn new_photo(owner_id: Uuid, content_hash: &str) -> NewPhoto {
    NewPhoto {
        voyage_id: owner_id,
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
    let owner = crate::db::repo::users::create(pool, None, None)
        .await
        .expect("create user")
        .id;
    sqlx::query("INSERT INTO voyages (id, owner_id, title) VALUES ($1, $1, 'Photo fixture')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("fixture voyage");
    owner
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
