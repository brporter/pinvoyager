use super::*;
use crate::photos::decode::testutil::ScratchDir;

#[test]
fn cache_expires_and_evicts_the_oldest_measurement() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let oldest = Uuid::now_v7();
    assert_eq!(cache.get(oldest, now), None);
    cache.insert(oldest, 10, now);
    for _ in 1..CAPACITY {
        cache.insert(Uuid::now_v7(), 0, now + Duration::from_secs(1));
    }
    cache.insert(oldest, 20, now);
    assert_eq!(cache.0.len(), CAPACITY);
    assert_eq!(cache.get(oldest, now), Some(20));
    let newest = Uuid::now_v7();
    cache.insert(newest, 30, now + Duration::from_secs(2));
    assert_eq!(cache.0.len(), CAPACITY);
    assert_eq!(cache.get(oldest, now + Duration::from_secs(2)), None);
    assert_eq!(cache.get(newest, now + TTL + Duration::from_secs(2)), None);
    cache.insert(newest, 40, now + TTL + Duration::from_secs(2));
    assert_eq!(cache.0.len(), 1);
}

#[sqlx::test]
async fn measurement_pages_hashes_and_refreshes_expired_totals(pool: PgPool) {
    let root = ScratchDir::new("account-storage");
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("user");
    let voyage = crate::photo_voyage::fixture_voyage(&pool, user.id).await;
    sqlx::query("INSERT INTO photos(id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format)
        SELECT gen_random_uuid(),$1,$2,lpad(to_hex(n),64,'0'),1,1,1,'jpeg' FROM generate_series(0,1000) n")
        .bind(user.id).bind(voyage).execute(&pool).await.expect("photos");
    let hash = ContentHash::try_from(format!("{:064x}", 1000)).expect("hash");
    let path = storage::original_path(root.path(), &hash);
    std::fs::create_dir_all(path.parent().expect("shard")).expect("directory");
    std::fs::write(&path, [0; 10]).expect("original");
    let cache = AccountStorage::default();
    assert_eq!(
        cache
            .totals(&pool, root.path(), &[user.id])
            .await
            .expect("first")[&user.id],
        10
    );
    std::fs::write(&path, [0; 20]).expect("changed original");
    assert_eq!(
        cache
            .totals(&pool, root.path(), &[user.id])
            .await
            .expect("cached")[&user.id],
        10
    );
    cache.0.lock().await.0.get_mut(&user.id).expect("entry").0 = Instant::now() - TTL;
    assert_eq!(
        cache
            .totals(&pool, root.path(), &[user.id])
            .await
            .expect("refreshed")[&user.id],
        20
    );
}

#[sqlx::test]
async fn measurement_errors_are_not_cached(pool: PgPool) {
    let root = ScratchDir::new("account-storage-error");
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("user");
    let voyage = crate::photo_voyage::fixture_voyage(&pool, user.id).await;
    sqlx::query("INSERT INTO photos(id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format)
        VALUES(gen_random_uuid(),$1,$2,$3,1,1,1,'jpeg')")
        .bind(user.id).bind(voyage).bind("a".repeat(64)).execute(&pool).await.expect("photo");
    let obstruction = root.path().join("originals");
    std::fs::write(&obstruction, [0]).expect("obstruction");
    let cache = AccountStorage::default();
    assert!(cache.totals(&pool, root.path(), &[user.id]).await.is_err());
    std::fs::remove_file(obstruction).expect("repair");
    assert_eq!(
        cache
            .totals(&pool, root.path(), &[user.id])
            .await
            .expect("retry")[&user.id],
        0
    );
}

#[sqlx::test]
async fn a_waiting_measurement_does_not_block_cached_or_unrelated_accounts(pool: PgPool) {
    let root = ScratchDir::new("storage-concurrency");
    let cache = AccountStorage::default();
    let cached = Uuid::now_v7();
    cache.0.lock().await.insert(cached, 42, Instant::now());
    let blocked_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool");
    let held = blocked_pool.acquire().await.expect("hold sole connection");
    let missing = [Uuid::now_v7()];
    let slow = cache.totals(&blocked_pool, root.path(), &missing);
    tokio::pin!(slow);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut slow)
            .await
            .is_err()
    );
    let hit = tokio::time::timeout(
        Duration::from_millis(200),
        cache.totals(&pool, root.path(), &[cached]),
    )
    .await
    .expect("cached reads do not wait for another account")
    .expect("cached");
    assert_eq!(hit[&cached], 42);
    let independent = Uuid::now_v7();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(1),
            cache.totals(&pool, root.path(), &[independent])
        )
        .await
        .expect("unrelated measurement proceeds")
        .expect("measured")[&independent],
        0
    );
    drop(held);
    slow.await.expect("original measurement completes");
}

#[sqlx::test]
async fn concurrent_requests_share_one_measurement_and_cancellation_releases_it(pool: PgPool) {
    let root = ScratchDir::new("storage-shared");
    let cache = AccountStorage::default();
    let ids = [Uuid::now_v7()];
    let blocked = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool");
    let held = blocked.acquire().await.expect("hold connection");
    let mut first = Box::pin(cache.totals(&blocked, root.path(), &ids));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut first)
            .await
            .is_err()
    );
    let closed =
        sqlx::postgres::PgPoolOptions::new().connect_lazy_with((*pool.connect_options()).clone());
    closed.close().await;
    let mut second = Box::pin(cache.totals(&closed, root.path(), &ids));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut second)
            .await
            .is_err()
    );
    drop(held);
    first.await.expect("first measurement");
    assert_eq!(
        second
            .await
            .expect("reuses first result without querying closed pool")[&ids[0]],
        0
    );

    let fresh = [Uuid::now_v7()];
    let held = blocked.acquire().await.expect("hold connection again");
    let mut canceled = Box::pin(cache.totals(&blocked, root.path(), &fresh));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut canceled)
            .await
            .is_err()
    );
    drop(canceled);
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        cache.totals(&pool, root.path(), &fresh),
    )
    .await
    .expect("canceled caller releases account lock")
    .expect("replacement measurement");
    assert_eq!(result[&fresh[0]], 0);
    drop(held);
}
