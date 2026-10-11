use super::*;
use crate::photos::VariantKind;
use crate::photos::decode::testutil::ScratchDir;

#[sqlx::test]
async fn cleanup_failure_is_retained_and_a_later_pass_removes_it(pool: PgPool) {
    let scratch = ScratchDir::new("cleanup-retry");
    let hash = ContentHash::try_from("a".repeat(64)).expect("hash");
    let blocked = storage::variant_path(scratch.path(), &hash, VariantKind::Thumb);
    std::fs::create_dir_all(&blocked).expect("directory cannot be unlinked as a file");
    let mut connection = pool.acquire().await.expect("connection");
    schedule(&mut connection, hash.as_str())
        .await
        .expect("schedule");
    schedule(&mut connection, hash.as_str())
        .await
        .expect("duplicate schedule is harmless");
    drop(connection);
    sweep(&pool, scratch.path())
        .await
        .expect("maintenance tolerates individual failures");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photo_cleanup")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
    std::fs::remove_dir(blocked).expect("clear obstruction");
    sweep(&pool, scratch.path()).await.expect("retry");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photo_cleanup")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn cleanup_rechecks_references_after_a_new_upload_adopts_content(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = crate::photo_voyage::fixture_voyage(&pool, owner.id).await;
    let hash = "b".repeat(64);
    let scratch = ScratchDir::new("cleanup-adopted");
    let path = storage::original_path(
        scratch.path(),
        &ContentHash::try_from(hash.clone()).expect("hash"),
    );
    std::fs::create_dir_all(path.parent().expect("shard parent")).expect("shards");
    std::fs::write(&path, b"original").expect("original");
    let mut connection = pool.acquire().await.expect("connection");
    schedule(&mut connection, &hash).await.expect("schedule");
    drop(connection);
    sqlx::query("INSERT INTO photos(id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format) VALUES(gen_random_uuid(),$1,$2,$3,8,1,1,'jpeg')")
        .bind(owner.id).bind(voyage).bind(&hash).execute(&pool).await.expect("new reference");
    clean(&pool, scratch.path(), &hash).await.expect("clean");
    assert_eq!(
        std::fs::read(path).expect("adopted original remains"),
        b"original"
    );
}
