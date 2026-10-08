use super::*;
use crate::photo_voyage;

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
            voyage_id: photo_voyage::fixture_voyage(pool, owner_id).await,
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

/// D11: deleting a voyage also deletes its owned photographs.
#[sqlx::test]
async fn deleting_a_voyage_deletes_its_photo_records(pool: PgPool) {
    let owner = a_user(&pool).await;
    let voyage = a_voyage(&pool, owner).await;
    let first = a_photo(&pool, owner, 0x11).await;
    let second = a_photo(&pool, owner, 0x12).await;

    assert_eq!(
        move_photos(&pool, voyage.id, owner, &[first, second])
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
                .is_none(),
            "the photograph record is deleted"
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
/// count tells the handler that the request was not honored in full so it
/// can roll back and return 403.
#[sqlx::test]
async fn attaching_another_users_photo_attaches_nothing(pool: PgPool) {
    let owner = a_user(&pool).await;
    let stranger = a_user(&pool).await;
    let voyage = a_voyage(&pool, owner).await;
    let theirs = a_photo(&pool, stranger, 0x21).await;

    assert_eq!(
        move_photos(&pool, voyage.id, owner, &[theirs])
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
        move_photos(&pool, voyage.id, owner, &[mine, theirs])
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
        move_photos(&pool, voyage.id, stranger, &[theirs])
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
                move_photos(&a, voyage_id, owner, &wanted),
                move_photos(&b, voyage_id, stranger, &wanted),
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
        move_photos(&pool, voyage.id, owner, &[screenshot, june, scan, april])
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

    move_photos(&pool, voyage.id, owner, &[first, second])
        .await
        .expect("attach");
    move_photos(&pool, voyage.id, owner, &[older])
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
        move_photos(&pool, voyage.id, owner, &[photo])
            .await
            .expect("attach"),
        1
    );
    assert_eq!(
        move_photos(&pool, voyage.id, owner, &[photo])
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
        move_photos(&pool, voyage.id, owner, &[])
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

    move_photos(&pool, voyage.id, owner, &[photo])
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
            .is_none(),
        "removing a photo deletes its record"
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
    move_photos(&pool, voyage.id, owner, &photos_in)
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

/// The documented behavior for a stale list: ids that are not in the
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
    move_photos(&pool, voyage.id, owner, &[a, b, c])
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
    move_photos(&pool, voyage.id, owner, &[a, b])
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
    move_photos(&pool, voyage.id, owner, &photos_in)
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
    move_photos(&pool, voyage.id, owner, &photos_in)
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
            voyage_id: photo_voyage::fixture_voyage(&pool, owner).await,
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

    move_photos(&pool, voyage.id, owner, &[placed, unplaced])
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
