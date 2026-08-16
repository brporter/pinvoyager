//! Content-addressed storage on disk.
//!
//! CONTRACT — signatures are fixed; implementation owned by the upload agent.
//!
//! Paths are derived from a SHA-256 of the content, sharded two levels so no
//! directory grows unbounded:
//!
//! ```text
//! {root}/originals/{aa}/{bb}/{full-hash}
//! {root}/variants/{aa}/{bb}/{full-hash}.{variant}.webp
//! ```
//!
//! **No user-supplied string ever reaches the filesystem.** That is what makes
//! path traversal a non-category here rather than something to defend against:
//! a hash is 64 hex characters and cannot contain a separator, a dot-dot, or a
//! null byte no matter what the uploader called their file.
//!
//! D8: these are blocking filesystem operations. Callers run them through
//! [`crate::blocking::run_blocking`] or `tokio::fs`.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::photos::{PhotoError, VariantKind};

/// Subdirectories beneath the configured storage root. Siblings rather than
/// nested so a variant sweep never has to walk past an original.
const ORIGINALS_DIR: &str = "originals";
const VARIANTS_DIR: &str = "variants";

/// Every variant is WebP (D9), so the extension is a constant rather than
/// something derived from the source format.
const VARIANT_EXTENSION: &str = "webp";

/// A hex SHA-256. Two shard levels of two characters give 256 directories at
/// each level — 65,536 leaf directories, which keeps any one of them small well
/// past any volume this service will see.
const HASH_HEX_LEN: usize = 64;
const SHARD_WIDTH: usize = 2;

/// Read size for streaming hashes. Large enough that the syscall overhead is
/// negligible, small enough that it is a rounding error against a 25 MB upload.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Hex-encoded SHA-256 of the original bytes. The dedup key and the path stem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHash(pub String);

impl ContentHash {
    /// Hashes a file's contents. CPU-bound on large files — call under
    /// `spawn_blocking`.
    pub fn of_file(path: &Path) -> Result<Self, PhotoError> {
        let mut file = std::fs::File::open(path).map_err(PhotoError::Storage)?;

        // Streamed in fixed chunks rather than `read_to_end`: the whole point
        // of hashing on the way past is that a 25 MB upload costs 64 KB of
        // resident memory, not 25 MB, and fifty concurrent ones cost 3 MB
        // rather than 1.2 GB.
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; HASH_CHUNK_BYTES];

        loop {
            let read = file.read(&mut buffer).map_err(PhotoError::Storage)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        Ok(ContentHash(hex::encode(hasher.finalize())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Splits a hash into its two shard components.
///
/// The `debug_assert` states the invariant the module header claims: the field
/// is public, so nothing in the type system stops a caller constructing a
/// `ContentHash` from an arbitrary string, and if one ever does, the traversal
/// guarantee is gone. Hashes reaching here come from [`ContentHash::of_file`]
/// or from the `content_hash` column, both of which are hex by construction.
fn shards(hash: &ContentHash) -> (&str, &str) {
    let hex = hash.as_str();

    debug_assert!(
        hex.len() == HASH_HEX_LEN && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "a ContentHash must be {HASH_HEX_LEN} hex characters; \
         a path derived from anything else is a traversal waiting to happen"
    );

    // `get` rather than slicing: a malformed hash must not panic in a release
    // build. The fallback shard is a dead end, which is the right failure —
    // the file will not be found and nothing outside the root is touched.
    (
        hex.get(0..SHARD_WIDTH).unwrap_or("__"),
        hex.get(SHARD_WIDTH..SHARD_WIDTH * 2).unwrap_or("__"),
    )
}

/// Where the original bytes live.
pub fn original_path(root: &Path, hash: &ContentHash) -> PathBuf {
    let (aa, bb) = shards(hash);
    root.join(ORIGINALS_DIR)
        .join(aa)
        .join(bb)
        .join(hash.as_str())
}

/// Where a derived variant lives.
pub fn variant_path(root: &Path, hash: &ContentHash, kind: VariantKind) -> PathBuf {
    let (aa, bb) = shards(hash);

    // Built as one filename rather than with `set_extension`, which would treat
    // the `.thumb` as the extension and replace it.
    let name = format!("{}.{}.{VARIANT_EXTENSION}", hash.as_str(), kind.as_str());

    root.join(VARIANTS_DIR).join(aa).join(bb).join(name)
}

/// Moves a completed upload into its content-addressed home.
///
/// Idempotent: if the destination already exists the bytes are identical by
/// construction, so the temporary file is simply discarded. That is the dedup
/// path, and it must not be an error — two users uploading the same photograph
/// is ordinary.
pub async fn commit_original(
    root: &Path,
    temp_path: &Path,
    hash: &ContentHash,
) -> Result<(), PhotoError> {
    let destination = original_path(root, hash);

    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(PhotoError::Storage)?;
    }

    // The dedup path. Checked before the rename rather than after a failure,
    // because on Unix `rename` silently overwrites and would give us no signal
    // at all.
    //
    // There is a race here and it is benign: two uploaders of the same bytes
    // can both see "absent" and both rename. `rename` is atomic, the loser's
    // bytes are byte-identical to the winner's, and any reader sees one
    // complete file throughout. That is the property content addressing buys.
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(PhotoError::Storage)?
    {
        discard_temp(temp_path).await;
        return Ok(());
    }

    match tokio::fs::rename(temp_path, &destination).await {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device(&err) => copy_across_filesystems(temp_path, &destination).await,
        Err(err) => Err(PhotoError::Storage(err)),
    }
}

/// `EXDEV`: the temporary file and the storage root are on different
/// filesystems, so there is no directory entry to move and `rename` cannot
/// work. Detected rather than assumed away because the default temp directory
/// is very often a tmpfs while the storage root is not, and the failure is a
/// hard error on every single upload rather than something that degrades.
fn is_cross_device(err: &std::io::Error) -> bool {
    // `raw_os_error` is checked alongside the kind because `CrossesDevices` is
    // a recent mapping and a platform that has not adopted it reports `Other`.
    // EXDEV is 18 on Linux and on macOS.
    matches!(err.kind(), std::io::ErrorKind::CrossesDevices) || err.raw_os_error() == Some(18)
}

/// Copy-and-unlink fallback for the cross-filesystem case.
///
/// The copy lands on a staging name *in the destination directory* and is then
/// renamed into place, so a reader never observes a partially written original
/// at a content-addressed path. Copying straight onto the final path would
/// leave a truncated file behind if the process died mid-copy, and its path
/// would claim a hash its contents do not have — the one thing content
/// addressing must never allow.
async fn copy_across_filesystems(temp_path: &Path, destination: &Path) -> Result<(), PhotoError> {
    tracing::warn!(
        temp = %temp_path.display(),
        destination = %destination.display(),
        "upload staging directory is on a different filesystem from the photo \
         storage root; every upload is being copied rather than renamed. Point \
         the upload temp directory inside the storage root."
    );

    let staging =
        destination.with_file_name(format!(".incoming-{}", uuid::Uuid::now_v7().simple()));

    tokio::fs::copy(temp_path, &staging)
        .await
        .map_err(PhotoError::Storage)?;

    if let Err(err) = tokio::fs::rename(&staging, destination).await {
        discard_temp(&staging).await;
        return Err(PhotoError::Storage(err));
    }

    discard_temp(temp_path).await;
    Ok(())
}

/// Removes an original and its variants.
///
/// **Only safe when no `photos` row still references the hash.** Deduplication
/// means two users can share one file; an unqualified unlink here would delete
/// another user's photograph. The reference check belongs to the caller, in the
/// same transaction as the row delete.
pub async fn remove_content(root: &Path, hash: &ContentHash) -> Result<(), PhotoError> {
    // Variants first. If the process dies between the two, what is left is an
    // original with no variants — which the worker can regenerate — rather than
    // variants with no original, which nothing can.
    for kind in VariantKind::ALL {
        remove_if_present(&variant_path(root, hash, kind)).await?;
    }

    remove_if_present(&original_path(root, hash)).await?;

    // The shard directories are deliberately left behind. There are only 65,536
    // of them, they cost an inode each, and removing one races with a
    // concurrent `commit_original` that has just created it and is about to
    // rename into it.
    Ok(())
}

/// Unlinks a path, treating "already gone" as success.
///
/// A photo deleted before its variants were generated has none, and a delete
/// retried after a partial failure finds some of them missing. Both are
/// ordinary, and neither should stop the rest of the removal.
async fn remove_if_present(path: &Path) -> Result<(), PhotoError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(PhotoError::Storage(err)),
    }
}

/// Drops a temporary file, logging rather than failing.
///
/// A leftover temp file is a wasted block, not a broken upload: the bytes are
/// already safely at their content-addressed path by the time this is called,
/// so turning a failed unlink into an error would fail an upload that
/// completely succeeded.
async fn discard_temp(path: &Path) {
    if let Err(err) = tokio::fs::remove_file(path).await
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            path = %path.display(),
            error = %err,
            "could not remove a temporary upload file"
        );
    }
}

#[cfg(test)]
mod tests {
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

        remove_content(&root, &content).await.expect("remove");

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

        remove_content(&root, &hash(ABC_SHA256))
            .await
            .expect("removing absent content is not an error");

        std::fs::remove_dir_all(&dir).ok();
    }
}
