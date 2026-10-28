use super::*;
use crate::photos::decode::testutil::ScratchDir;

fn aged_file(path: &Path, when: SystemTime) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directories");
    std::fs::write(path, b"bytes").expect("file");
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_times(std::fs::FileTimes::new().set_modified(when))
        .expect("mtime");
}

#[test]
fn only_canonical_content_paths_and_shards_are_recognized() {
    let root = Path::new("store");
    let hash = ContentHash::try_from("ab".repeat(32)).expect("hash");
    for (tree, path) in [
        ("originals", storage::original_path(root, &hash)),
        (
            "variants",
            storage::variant_path(root, &hash, VariantKind::Thumb),
        ),
    ] {
        assert_eq!(content_hash(root, &path, tree), Some(hash.clone()));
    }
    for (tree, name) in [
        ("originals", "notes".to_string()),
        ("variants", hash.as_str().to_owned()),
        ("variants", format!("{}.thumb.png", hash.as_str())),
        ("variants", format!("{}.other.webp", hash.as_str())),
        ("variants", "bad.thumb.webp".into()),
        ("originals", hash.as_str().to_uppercase()),
    ] {
        assert!(content_hash(root, &root.join(tree).join("ab/ab").join(name), tree).is_none());
    }
    assert!(
        content_hash(
            root,
            &root.join("originals/00/00").join(hash.as_str()),
            "originals"
        )
        .is_none()
    );
    assert!(content_hash(root, Path::new("/"), "originals").is_none());
    for name in ["00", "af", "ff"] {
        assert!(is_shard(name.as_ref()));
    }
    for name in ["", "a", "abc", "g0", "AF"] {
        assert!(!is_shard(name.as_ref()));
    }
}

#[test]
fn removal_requires_age_strictly_beyond_the_cutoff() {
    let scratch = ScratchDir::new("orphan-age");
    let hash = ContentHash::try_from("b".repeat(64)).expect("hash");
    let path = storage::original_path(scratch.path(), &hash);
    aged_file(&path, SystemTime::now() - MINIMUM_AGE);
    let modified = std::fs::metadata(&path)
        .expect("metadata")
        .modified()
        .expect("mtime");
    remove_older_than(scratch.path(), &hash, modified).expect("exactly one day");
    assert!(path.exists());
    remove_older_than(scratch.path(), &hash, modified + Duration::from_nanos(1))
        .expect("more than one day");
    assert!(!path.exists());
    remove_older_than(scratch.path(), &hash, modified).expect("already missing");
}

#[cfg(unix)]
#[sqlx::test]
async fn scans_and_removals_preserve_symlinks_unrelated_files_and_directories(pool: PgPool) {
    use std::os::unix::fs::symlink;
    let scratch = ScratchDir::new("orphan-links");
    let outside = ScratchDir::new("orphan-link-target");
    let hash = ContentHash::try_from("c".repeat(64)).expect("hash");
    let old = SystemTime::now() - MINIMUM_AGE * 2;
    let original = storage::original_path(scratch.path(), &hash);
    aged_file(&original, old);
    let outside_variant = outside
        .path()
        .join(format!("cc/cc/{}.thumb.webp", hash.as_str()));
    aged_file(&outside_variant, old);
    symlink(outside.path(), scratch.path().join("variants")).expect("tree symlink");
    sweep(&pool, scratch.path()).await.expect("scan");
    assert!(!original.exists());
    assert!(
        outside_variant.exists(),
        "original cleanup must not follow the variants tree symlink"
    );
    std::fs::remove_file(scratch.path().join("variants")).expect("remove link");
    let variant = storage::variant_path(scratch.path(), &hash, VariantKind::Thumb);
    std::fs::create_dir_all(variant.parent().expect("parent")).expect("shards");
    symlink(&outside_variant, &variant).expect("file symlink");
    let directory = storage::variant_path(scratch.path(), &hash, VariantKind::Full);
    std::fs::create_dir(&directory).expect("directory with image name");
    let unrelated = variant.parent().expect("parent").join("operator-notes.txt");
    aged_file(&unrelated, old);
    let wrong_shard = scratch.path().join("originals/00/00").join(hash.as_str());
    aged_file(&wrong_shard, old);
    symlink(outside.path(), scratch.path().join("originals/aa")).expect("shard symlink");
    remove_older_than(scratch.path(), &hash, SystemTime::now()).expect("safe removal");
    sweep(&pool, scratch.path()).await.expect("safe scan");
    assert!(
        variant
            .symlink_metadata()
            .expect("link survives")
            .file_type()
            .is_symlink()
    );
    assert!(directory.is_dir());
    assert!(unrelated.exists());
    assert!(wrong_shard.exists());
    assert!(outside_variant.exists());
}

#[sqlx::test]
async fn scan_errors_propagate_and_database_errors_leave_orphans_for_retry(pool: PgPool) {
    let scratch = ScratchDir::new("orphan-errors");
    sweep(&pool, scratch.path()).await.expect("absent trees");
    std::fs::write(scratch.path().join("originals"), b"obstruction").expect("obstruction");
    assert!(sweep(&pool, scratch.path()).await.is_err());
    std::fs::remove_file(scratch.path().join("originals")).expect("remove obstruction");
    let hash = ContentHash::try_from("e".repeat(64)).expect("hash");
    let path = storage::original_path(scratch.path(), &hash);
    aged_file(&path, SystemTime::now() - MINIMUM_AGE * 2);
    pool.close().await;
    sweep(&pool, scratch.path())
        .await
        .expect("individual errors are logged");
    assert!(path.exists());
}

#[sqlx::test]
async fn orphan_cleanup_rechecks_references_and_file_age_after_waiting_for_the_lock(pool: PgPool) {
    let scratch = ScratchDir::new("orphan-races");
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = crate::photo_voyage::fixture_voyage(&pool, user.id).await;
    for (index, action) in ["adopt", "refresh", "remove"].into_iter().enumerate() {
        let hash = ContentHash::try_from(format!("{index:064x}")).expect("hash");
        let path = storage::original_path(scratch.path(), &hash);
        aged_file(&path, SystemTime::now() - MINIMUM_AGE * 2);
        let mut upload = pool.begin().await.expect("upload transaction");
        crate::db::repo::photos::lock_content(&mut upload, hash.as_str())
            .await
            .expect("content lock");
        let task_pool = pool.clone();
        let root = scratch.path().to_owned();
        let scan = tokio::spawn(async move { sweep(&task_pool, &root).await });
        tokio::time::timeout(Duration::from_secs(2),async {
            loop {
                let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event='advisory')")
                    .fetch_one(&pool).await.expect("lock wait");
                if waiting { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("scanner waits for upload lock");
        match action {
            "adopt" => {
                sqlx::query("INSERT INTO photos(id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format) VALUES(gen_random_uuid(),$1,$2,$3,8,1,1,'jpeg')")
                .bind(user.id).bind(voyage).bind(hash.as_str()).execute(&mut *upload).await.expect("adopt bytes");
            }
            "refresh" => aged_file(&path, SystemTime::now()),
            _ => std::fs::remove_file(&path).expect("another cleanup won"),
        }
        upload.commit().await.expect("upload committed");
        scan.await.expect("scan task").expect("scan result");
        assert_eq!(path.exists(), action != "remove");
    }
}

#[cfg(unix)]
#[test]
fn non_unicode_and_root_paths_cannot_be_interpreted_as_content_hashes() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};
    let invalid = OsString::from_vec(vec![0xff, 0xfe]);
    assert!(!is_shard(&invalid));
    assert!(content_hash(Path::new("/storage"), Path::new("/"), "originals").is_none());
    assert!(
        content_hash(
            Path::new("/storage"),
            &Path::new("/storage/originals/aa/aa").join(invalid),
            "originals"
        )
        .is_none()
    );
}
