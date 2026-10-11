//! Content-addressed storage on disk.
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
pub struct ContentHash(String);

impl TryFrom<String> for ContentHash {
    type Error = crate::AppError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != HASH_HEX_LEN
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(crate::AppError::Internal("invalid stored SHA-256".into()));
        }
        Ok(Self(value))
    }
}

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

/// Construction guarantees ASCII hex, so these byte offsets are valid UTF-8 boundaries.
fn shards(hash: &ContentHash) -> (&str, &str) {
    (
        &hash.0[..SHARD_WIDTH],
        &hash.0[SHARD_WIDTH..SHARD_WIDTH * 2],
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
#[tracing::instrument(name = "photo.store", skip_all, err)]
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
        Err(err) => recover_failed_rename(temp_path, &destination, err).await,
    }
}

/// Selects the safe response to a failed rename: copy across filesystems, or
/// return every other filesystem error to the caller.
async fn recover_failed_rename(
    temp_path: &Path,
    destination: &Path,
    err: std::io::Error,
) -> Result<(), PhotoError> {
    if is_cross_device(&err) {
        copy_across_filesystems(temp_path, destination).await
    } else {
        Err(PhotoError::Storage(err))
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
pub fn remove_content(root: &Path, hash: &ContentHash) -> Result<(), PhotoError> {
    // Variants first. If the process dies between the two, what is left is an
    // original with no variants — which the worker can regenerate — rather than
    // variants with no original, which nothing can.
    for kind in VariantKind::ALL {
        remove_if_present(&variant_path(root, hash, kind))?;
    }

    remove_if_present(&original_path(root, hash))?;

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
fn remove_if_present(path: &Path) -> Result<(), PhotoError> {
    match std::fs::remove_file(path) {
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
#[path = "storage_tests.rs"]
mod tests;
