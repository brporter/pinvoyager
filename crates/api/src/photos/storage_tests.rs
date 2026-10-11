use super::*;

/// SHA-256 of the empty input, which is a value that can be checked against
/// any other implementation.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// SHA-256 of `abc`, from FIPS 180-4's own example.
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn hash(hex: &str) -> ContentHash {
    ContentHash(hex.to_owned())
}

/// An isolated directory per test. `tempfile` is not a dependency, and a
/// v7 uuid is unique enough for the purpose.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("pinvoyager-storage-tests")
        .join(format!("{name}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn an_original_path_is_sharded_two_levels_deep() {
    let root = Path::new("/srv/data");
    let path = original_path(root, &hash(ABC_SHA256));

    assert_eq!(
        path,
        Path::new("/srv/data/originals/ba/78/").join(ABC_SHA256)
    );
}

#[test]
fn a_variant_path_carries_the_kind_and_the_webp_extension() {
    let root = Path::new("/srv/data");

    for (kind, suffix) in [
        (VariantKind::Thumb, "thumb"),
        (VariantKind::Display, "display"),
        (VariantKind::Full, "full"),
    ] {
        let path = variant_path(root, &hash(ABC_SHA256), kind);
        assert_eq!(
            path,
            Path::new("/srv/data/variants/ba/78/").join(format!("{ABC_SHA256}.{suffix}.webp"))
        );
    }
}

/// The shard directories come from the *hash*, so two different hashes
/// sharing a first byte share a directory and nothing else.
#[test]
fn originals_and_variants_live_in_sibling_trees() {
    let root = Path::new("data");
    let original = original_path(root, &hash(EMPTY_SHA256));
    let variant = variant_path(root, &hash(EMPTY_SHA256), VariantKind::Thumb);

    assert!(original.starts_with(Path::new("data/originals")));
    assert!(variant.starts_with(Path::new("data/variants")));
    assert!(original.starts_with(root) && variant.starts_with(root));
}

/// The property the module header claims. A path built from a hash has
/// exactly five components below the root and none of them is `..`, so
/// there is nothing to escape with — whatever the uploader called the file.
#[test]
fn no_component_of_a_derived_path_can_escape_the_root() {
    let root = Path::new("/srv/data");

    for path in [
        original_path(root, &hash(ABC_SHA256)),
        variant_path(root, &hash(ABC_SHA256), VariantKind::Full),
    ] {
        assert!(path.starts_with(root));
        assert!(
            !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
            "{path:?} contains a parent-directory component"
        );
    }
}

#[test]
fn hashing_an_empty_file_gives_the_known_digest() {
    let dir = scratch_dir("hash-empty");
    let path = dir.join("empty");
    std::fs::write(&path, b"").expect("write");

    assert_eq!(
        ContentHash::of_file(&path).expect("hash").as_str(),
        EMPTY_SHA256
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hashing_matches_the_fips_test_vector() {
    let dir = scratch_dir("hash-abc");
    let path = dir.join("abc");
    std::fs::write(&path, b"abc").expect("write");

    assert_eq!(
        ContentHash::of_file(&path).expect("hash").as_str(),
        ABC_SHA256
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The streaming property: a file several times the read chunk must hash
/// identically to the same bytes hashed in one go. A bug in the loop —
/// hashing the whole buffer rather than the bytes actually read, or
/// stopping at the first short read — shows up here and nowhere else.
#[test]
fn a_file_larger_than_the_read_chunk_hashes_correctly() {
    let dir = scratch_dir("hash-large");
    let path = dir.join("large");

    // Not a repeating byte: a stuck offset would still produce the right
    // digest on uniform content.
    let bytes: Vec<u8> = (0..HASH_CHUNK_BYTES * 3 + 137)
        .map(|i| (i % 251) as u8)
        .collect();
    std::fs::write(&path, &bytes).expect("write");

    let expected = hex::encode(Sha256::digest(&bytes));
    assert_eq!(
        ContentHash::of_file(&path).expect("hash").as_str(),
        expected
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hashing_a_missing_file_is_a_storage_error() {
    let missing = scratch_dir("hash-missing").join("nope");
    assert!(matches!(
        ContentHash::of_file(&missing),
        Err(PhotoError::Storage(_))
    ));
}

#[tokio::test]
async fn committing_creates_the_shard_directories_and_moves_the_file() {
    let dir = scratch_dir("commit");
    let root = dir.join("store");
    let temp = dir.join("upload.tmp");
    std::fs::write(&temp, b"abc").expect("write");

    let content = ContentHash::of_file(&temp).expect("hash");
    commit_original(&root, &temp, &content)
        .await
        .expect("commit");

    let destination = original_path(&root, &content);
    assert_eq!(std::fs::read(&destination).expect("read"), b"abc");
    assert!(!temp.exists(), "the temporary file is consumed");

    std::fs::remove_dir_all(&dir).ok();
}

/// T3: uploading identical bytes twice stores one file. The second commit
/// must succeed — it is the ordinary case of two users owning the same
/// photograph, not an error to report.
#[tokio::test]
async fn committing_the_same_content_twice_is_not_an_error() {
    let dir = scratch_dir("commit-dedup");
    let root = dir.join("store");

    let first = dir.join("first.tmp");
    let second = dir.join("second.tmp");
    std::fs::write(&first, b"abc").expect("write");
    std::fs::write(&second, b"abc").expect("write");

    let content = ContentHash::of_file(&first).expect("hash");
    commit_original(&root, &first, &content)
        .await
        .expect("first commit");
    commit_original(&root, &second, &content)
        .await
        .expect("second commit");

    assert_eq!(
        std::fs::read(original_path(&root, &content)).expect("read"),
        b"abc"
    );
    assert!(
        !second.exists(),
        "the duplicate temporary file is discarded, not left to accumulate"
    );

    // One file on disk, not two.
    let shard = original_path(&root, &content)
        .parent()
        .expect("shard dir")
        .to_path_buf();
    assert_eq!(
        std::fs::read_dir(&shard).expect("read shard").count(),
        1,
        "identical bytes produce one file"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn removing_content_takes_the_original_and_every_variant() {
    let dir = scratch_dir("remove");
    let root = dir.join("store");

    let temp = dir.join("upload.tmp");
    std::fs::write(&temp, b"abc").expect("write");
    let content = ContentHash::of_file(&temp).expect("hash");
    commit_original(&root, &temp, &content)
        .await
        .expect("commit");

    for kind in VariantKind::ALL {
        let path = variant_path(&root, &content, kind);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"variant").expect("write variant");
    }

    remove_content(&root, &content).expect("remove");

    assert!(!original_path(&root, &content).exists());
    for kind in VariantKind::ALL {
        assert!(!variant_path(&root, &content, kind).exists());
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// Removal is retried after a partial failure and runs against photos whose
/// variants were never generated, so missing files are the normal case.
#[tokio::test]
async fn removing_content_that_is_already_gone_succeeds() {
    let dir = scratch_dir("remove-absent");
    let root = dir.join("store");

    remove_content(&root, &hash(ABC_SHA256)).expect("removing absent content is not an error");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hashing_a_directory_reports_a_storage_read_error() {
    let dir = scratch_dir("hash-directory");

    assert!(matches!(
        ContentHash::of_file(&dir),
        Err(PhotoError::Storage(_))
    ));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn committing_reports_when_the_temporary_file_is_missing() {
    let dir = scratch_dir("commit-missing-temp");
    let root = dir.join("store");
    let missing = dir.join("missing.tmp");

    let error = commit_original(&root, &missing, &hash(ABC_SHA256))
        .await
        .expect_err("renaming a missing upload must fail");

    assert!(matches!(error, PhotoError::Storage(_)));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn committing_reports_when_shard_directories_cannot_be_created() {
    let dir = scratch_dir("commit-parent-error");
    let root = dir.join("store");
    let originals = root.join(ORIGINALS_DIR);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(&originals, b"blocks the originals directory").expect("create blocker");
    let temp = dir.join("upload.tmp");
    std::fs::write(&temp, b"abc").expect("write upload");

    let error = commit_original(&root, &temp, &hash(ABC_SHA256))
        .await
        .expect_err("creating a child beneath a regular file must fail");

    assert!(matches!(error, PhotoError::Storage(_)));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn removal_reports_filesystem_errors_for_directories() {
    let dir = scratch_dir("remove-directory");
    let unexpected_directory = dir.join("not-a-file");
    std::fs::create_dir(&unexpected_directory).expect("create directory");

    let error =
        remove_if_present(&unexpected_directory).expect_err("remove_file must reject a directory");

    assert!(matches!(error, PhotoError::Storage(_)));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn discarding_a_directory_is_best_effort() {
    let dir = scratch_dir("discard-directory");
    let unexpected_directory = dir.join("not-a-file");
    std::fs::create_dir(&unexpected_directory).expect("create directory");

    discard_temp(&unexpected_directory).await;

    assert!(unexpected_directory.is_dir());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn cross_filesystem_copy_installs_the_destination_and_consumes_the_source() {
    let dir = scratch_dir("cross-filesystem-copy");
    let source = dir.join("upload.tmp");
    let destination = dir.join("store/original");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create destination directory");
    std::fs::write(&source, b"original bytes").expect("write source");

    copy_across_filesystems(&source, &destination)
        .await
        .expect("copy-and-rename should install the original");

    assert_eq!(
        std::fs::read(&destination).expect("read installed file"),
        b"original bytes"
    );
    assert!(
        !source.exists(),
        "successful copy consumes the staging upload"
    );
    assert_eq!(
        std::fs::read_dir(destination.parent().expect("valid test fixture"))
            .expect("read destination directory")
            .count(),
        1,
        "the staging name is renamed away"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn cross_filesystem_copy_cleans_its_staging_file_when_installation_fails() {
    let dir = scratch_dir("cross-filesystem-copy-rename-error");
    let source = dir.join("upload.tmp");
    let destination = dir.join("store/original");
    std::fs::create_dir_all(&destination).expect("a directory blocks the final rename");
    std::fs::write(&source, b"original bytes").expect("write source");

    let error = copy_across_filesystems(&source, &destination)
        .await
        .expect_err("renaming a staging file over a directory must fail");

    assert!(matches!(error, PhotoError::Storage(_)));
    assert_eq!(
        std::fs::read_dir(&destination)
            .expect("read blocked destination directory")
            .count(),
        0,
        "the failed staging file is removed"
    );
    assert!(source.exists(), "a failed copy leaves the upload available");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn cross_filesystem_copy_reports_a_missing_source() {
    let dir = scratch_dir("cross-filesystem-copy-source-error");
    let source = dir.join("missing.tmp");
    let destination = dir.join("store/original");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create destination directory");

    assert!(matches!(
        copy_across_filesystems(&source, &destination).await,
        Err(PhotoError::Storage(_))
    ));
    assert!(!destination.exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_cross_device_rename_error_selects_the_copy_fallback() {
    let dir = scratch_dir("cross-device-rename-fallback");
    let source = dir.join("upload.tmp");
    let destination = dir.join("store/original");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create destination directory");
    std::fs::write(&source, b"original bytes").expect("write source");
    let cross_device = std::io::Error::from(std::io::ErrorKind::CrossesDevices);

    recover_failed_rename(&source, &destination, cross_device)
        .await
        .expect("cross-device rename uses the safe copy fallback");

    assert_eq!(
        std::fs::read(&destination).expect("valid test fixture"),
        b"original bytes"
    );
    assert!(!source.exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stored_hashes_reject_paths_short_values_and_non_lowercase_hex() {
    for invalid in [
        "",
        "../../etc/passwd",
        &"a".repeat(63),
        &"a".repeat(65),
        &"A".repeat(64),
        &"g".repeat(64),
    ] {
        assert!(
            ContentHash::try_from(invalid.to_owned()).is_err(),
            "accepted {invalid}"
        );
    }
    assert_eq!(
        ContentHash::try_from(ABC_SHA256.to_owned())
            .expect("valid SHA-256")
            .as_str(),
        ABC_SHA256
    );
}
