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
    sqlx::query("UPDATE photo_cleanup SET next_attempt_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .expect("retry becomes due");
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

#[sqlx::test]
async fn failing_old_records_do_not_starve_newer_cleanup(pool: PgPool) {
    let scratch = ScratchDir::new("cleanup-fairness");
    for index in 0..1000 {
        let hash = ContentHash::try_from(format!("{index:064x}")).expect("hash");
        std::fs::create_dir_all(storage::variant_path(
            scratch.path(),
            &hash,
            VariantKind::Thumb,
        ))
        .expect("obstruction");
        sqlx::query(
            "INSERT INTO photo_cleanup(content_hash,created_at) VALUES($1,now()-interval '1 hour')",
        )
        .bind(hash.as_str())
        .execute(&pool)
        .await
        .expect("old cleanup");
    }
    let newer = "f".repeat(64);
    sqlx::query("INSERT INTO photo_cleanup(content_hash) VALUES($1)")
        .bind(&newer)
        .execute(&pool)
        .await
        .expect("new cleanup");
    sweep(&pool, scratch.path()).await.expect("first pass");
    sweep(&pool, scratch.path()).await.expect("second pass");
    let remains: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM photo_cleanup WHERE content_hash=$1)")
            .bind(newer)
            .fetch_one(&pool)
            .await
            .expect("remaining");
    assert!(
        !remains,
        "persistent failures must leave room for newer cleanup"
    );
}

#[sqlx::test]
async fn sweep_removes_only_stale_unlocked_upload_files(pool: PgPool) {
    use std::{
        fs::{File, FileTimes},
        time::{Duration, SystemTime},
    };
    let scratch = ScratchDir::new("stale-uploads");
    let tmp = scratch.path().join("tmp");
    std::fs::create_dir(&tmp).expect("temporary directory");
    let old = SystemTime::now() - Duration::from_secs(48 * 60 * 60);
    let stale = tmp.join(format!("{}.part", uuid::Uuid::now_v7()));
    let active = tmp.join(format!("{}.part", uuid::Uuid::now_v7()));
    let recent = tmp.join(format!("{}.part", uuid::Uuid::now_v7()));
    let unrelated = tmp.join("operator-notes.part");
    for path in [&stale, &active, &unrelated] {
        File::create(path)
            .expect("file")
            .set_times(FileTimes::new().set_modified(old))
            .expect("age");
    }
    File::create(&recent).expect("recent file");
    let locked = File::options()
        .read(true)
        .write(true)
        .open(&active)
        .expect("active file");
    locked.lock().expect("hold upload lock");
    let directory = tmp.join(format!("{}.part", uuid::Uuid::now_v7()));
    std::fs::create_dir(&directory).expect("unrelated directory");
    sweep(&pool, scratch.path()).await.expect("sweep");
    assert!(!stale.exists(), "stale abandoned uploads are removed");
    assert!(
        active.exists(),
        "an active upload survives regardless of age"
    );
    assert!(recent.exists());
    assert!(unrelated.exists());
    assert!(directory.exists());
    drop(locked);
    sweep(&pool, scratch.path())
        .await
        .expect("sweep unlocked file");
    assert!(!active.exists());
}

fn write_aged(path: &Path, age: std::time::Duration) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directories");
    std::fs::write(path, b"stored bytes").expect("file");
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("file")
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now() - age))
        .expect("mtime");
}

#[sqlx::test]
async fn orphan_sweep_removes_old_unreferenced_files_only(pool: PgPool) {
    use std::time::Duration;
    let scratch = ScratchDir::new("orphans");
    let old = Duration::from_secs(48 * 60 * 60);
    let hash = ContentHash::try_from("c".repeat(64)).expect("hash");
    let original = storage::original_path(scratch.path(), &hash);
    let thumb = storage::variant_path(scratch.path(), &hash, VariantKind::Thumb);
    let recent = storage::variant_path(scratch.path(), &hash, VariantKind::Display);
    write_aged(&original, old);
    write_aged(&thumb, old);
    write_aged(&recent, Duration::ZERO);
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = crate::photo_voyage::fixture_voyage(&pool, owner.id).await;
    let shared = ContentHash::try_from("d".repeat(64)).expect("hash");
    let retained = storage::original_path(scratch.path(), &shared);
    write_aged(&retained, old);
    sqlx::query("INSERT INTO photos(id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format) VALUES(gen_random_uuid(),$1,$2,$3,8,1,1,'jpeg')")
        .bind(owner.id).bind(voyage).bind(shared.as_str()).execute(&pool).await.expect("reference");
    // A variant without an original must also be discovered.
    let alone = storage::variant_path(
        scratch.path(),
        &ContentHash::try_from("e".repeat(64)).expect("hash"),
        VariantKind::Full,
    );
    write_aged(&alone, old);
    let unrelated = original.parent().expect("parent").join("operator-notes");
    write_aged(&unrelated, old);
    sweep(&pool, scratch.path())
        .await
        .expect("sweep without cleanup records");
    assert!(!original.exists(), "old orphaned original must be removed");
    assert!(!thumb.exists());
    assert!(!alone.exists());
    assert!(recent.exists(), "each file needs its own age check");
    assert!(
        retained.exists(),
        "referenced content survives regardless of age"
    );
    assert!(unrelated.exists());
}
