use super::*;
use crate::photos::decode::testutil::ScratchDir;

#[tokio::test]
async fn a_blocking_probe_keeps_its_upload_alive_until_it_finishes() {
    let scratch = ScratchDir::new("temporary-lease");
    let (lease, writer) = TemporaryUpload::create(scratch.path())
        .await
        .expect("temporary");
    let path = lease.path().to_owned();
    let probe = lease.clone();
    drop(writer);
    drop(lease);
    assert!(path.exists());
    let independent = File::options()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    assert!(matches!(
        independent.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(independent);
    drop(probe);
    tokio::time::timeout(Duration::from_secs(2), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("last lease removes the file");
}

#[tokio::test]
async fn creation_and_sweep_errors_are_returned_and_missing_directories_are_allowed() {
    let scratch = ScratchDir::new("temporary-errors");
    assert!(
        TemporaryUpload::create(&scratch.path().join("missing"))
            .await
            .is_err()
    );
    sweep(scratch.path()).await.expect("missing tmp is allowed");
    std::fs::write(scratch.path().join("tmp"), b"not a directory").expect("obstruction");
    assert!(sweep(scratch.path()).await.is_err());
    assert!(remove_if_present(scratch.path()).is_err());
}

#[cfg(unix)]
#[test]
fn sweeps_skip_symlinks_and_files_with_other_names_or_extensions() {
    use std::os::unix::fs::symlink;
    let scratch = ScratchDir::new("temporary-links");
    let target = scratch.path().join("original");
    std::fs::write(&target, b"keep").expect("target");
    let link = scratch
        .path()
        .join(format!("{}.part", uuid::Uuid::now_v7()));
    symlink(&target, &link).expect("symlink");
    let now = SystemTime::now();
    sweep_directory(scratch.path(), now + STALE_AFTER * 2).expect("sweep");
    assert!(
        link.symlink_metadata()
            .expect("symlink remains")
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(target).expect("target remains"), b"keep");
}
