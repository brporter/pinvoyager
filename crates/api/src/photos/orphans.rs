//! Finds old content files without photo rows after failed or canceled inserts.

use super::{
    VariantKind, cleanup,
    storage::{self, ContentHash},
};
use crate::error::{AppError, AppResult};
use sqlx::PgPool;
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const MINIMUM_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Walks the two content trees without following symlinks. Directory entries are
/// streamed; only shard directory paths are retained between reads. Missing trees
/// succeed. Scan errors propagate; individual cleanup failures are logged and
/// retried by a later scan. Filesystem operations run through Tokio's blocking pool.
pub(super) async fn sweep(pool: &PgPool, root: &Path) -> AppResult<()> {
    let cutoff = SystemTime::now() - MINIMUM_AGE;
    for tree in ["originals", "variants"] {
        let mut directories = vec![(root.join(tree), 0)];
        while let Some((directory, depth)) = directories.pop() {
            let metadata = match tokio::fs::symlink_metadata(&directory).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(internal(error)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            let mut entries = tokio::fs::read_dir(directory).await.map_err(internal)?;
            while let Some(entry) = entries.next_entry().await.map_err(internal)? {
                let kind = entry.file_type().await.map_err(internal)?;
                let path = entry.path();
                if kind.is_dir() && depth < 2 && is_shard(&entry.file_name()) {
                    directories.push((path, depth + 1));
                    continue;
                }
                if !kind.is_file() || depth != 2 {
                    continue;
                }
                let Some(hash) = content_hash(root, &path, tree) else {
                    continue;
                };
                let metadata = match tokio::fs::symlink_metadata(&path).await {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(internal(error)),
                };
                if metadata.is_file()
                    && metadata.modified().map_err(internal)? < cutoff
                    && let Err(error) =
                        cleanup::clean_content(pool, root, hash.as_str(), Some(cutoff)).await
                {
                    tracing::warn!(%error, "orphaned content cleanup will be retried on the next scan");
                }
            }
        }
    }
    Ok(())
}

fn is_shard(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.len() == 2
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Recognizes only canonical content paths, excluding temporary files, unknown
/// variant names, and files placed in the wrong shard directory.
fn content_hash(root: &Path, path: &Path, tree: &str) -> Option<ContentHash> {
    let name = path.file_name()?.to_str()?;
    let (hash, expected) = if tree == "originals" {
        let hash = ContentHash::try_from(name.to_owned()).ok()?;
        let expected = storage::original_path(root, &hash);
        (hash, expected)
    } else {
        let (hash, variant) = name.split_once('.')?;
        let hash = ContentHash::try_from(hash.to_owned()).ok()?;
        let variant = VariantKind::parse(variant.strip_suffix(".webp")?)?;
        let expected = storage::variant_path(root, &hash, variant);
        (hash, expected)
    };
    (path == expected).then_some(hash)
}

/// Rechecks each file's age under the content lock. Recent files, symlinks, and
/// directories survive; missing files succeed. Other filesystem errors propagate.
/// The caller must first check references and hold the lock until this returns.
pub(super) fn remove_older_than(
    root: &Path,
    hash: &ContentHash,
    cutoff: SystemTime,
) -> io::Result<()> {
    let paths: [PathBuf; 4] = [
        storage::original_path(root, hash),
        storage::variant_path(root, hash, VariantKind::Thumb),
        storage::variant_path(root, hash, VariantKind::Display),
        storage::variant_path(root, hash, VariantKind::Full),
    ];
    'files: for path in paths {
        // A candidate from the originals tree must not cause removal through a
        // symlink in the sibling variants tree (or either tree's shard folders).
        for directory in path.ancestors().skip(1).take(3) {
            match std::fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => continue 'files,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue 'files,
                Err(error) => return Err(error),
            }
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if metadata.is_file() && metadata.modified()? < cutoff {
            match std::fs::remove_file(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                result => result?,
            }
        }
    }
    Ok(())
}

fn internal(error: io::Error) -> AppError {
    AppError::Internal(Box::new(error))
}

#[cfg(test)]
#[path = "orphans_tests.rs"]
mod tests;
