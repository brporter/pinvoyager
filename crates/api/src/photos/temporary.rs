//! Temporary uploads retain a file lock while in use and remove their path on drop.
//! Maintenance removes abandoned, unlocked `.part` files older than one day.

use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{blocking::run_blocking_infallible, error::AppResult};

const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
pub(crate) struct TemporaryUpload(Arc<Lease>);

struct Lease {
    path: PathBuf,
    file: Option<File>,
    runtime: tokio::runtime::Handle,
}

impl TemporaryUpload {
    /// Creates and locks a unique file off the runtime worker threads. The writer
    /// shares its file description with the lease; clones keep the file alive during blocking probes.
    /// Creation, locking, and descriptor-cloning errors propagate to the caller.
    pub(crate) async fn create(directory: &Path) -> AppResult<(Self, tokio::fs::File)> {
        let path = directory.join(format!("{}.part", uuid::Uuid::now_v7()));
        let runtime = tokio::runtime::Handle::current();
        let (lease, writer) = run_blocking_infallible(move || {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)?;
            let lease = Self(Arc::new(Lease {
                path,
                file: Some(file),
                runtime,
            }));
            let file = lease.0.file.as_ref().expect("new lease owns its file");
            file.lock()?;
            let writer = file.try_clone()?;
            Ok::<_, io::Error>((lease, writer))
        })
        .await?
        .map_err(|error| crate::AppError::Internal(Box::new(error)))?;
        Ok((lease, tokio::fs::File::from_std(writer)))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0.path
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let path = self.path.clone();
        let file = self.file.take();
        // Keep the lock until removal finishes. Runtime shutdown may prevent this
        // task from starting; the next maintenance sweep handles that abandoned file.
        self.runtime.spawn_blocking(move || {
            let _lock = file;
            if let Err(error) = remove_if_present(&path) {
                tracing::warn!(%error, "temporary upload cleanup will be retried by maintenance");
            }
        });
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Removes stale UUID-named regular files in `tmp`, skipping symlinks, recent
/// files, and files locked by any API process. Missing directories succeed.
/// Other filesystem errors return an error so maintenance can log and retry.
pub(crate) async fn sweep(root: &Path) -> AppResult<()> {
    let directory = root.join("tmp");
    run_blocking_infallible(move || sweep_directory(&directory, SystemTime::now()))
        .await?
        .map_err(|error| crate::AppError::Internal(Box::new(error)))?;
    Ok(())
}

fn sweep_directory(directory: &Path, now: SystemTime) -> io::Result<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("part")
            || path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| uuid::Uuid::parse_str(stem).ok())
                .is_none()
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
        if now
            .duration_since(file.metadata()?.modified()?)
            .unwrap_or_default()
            >= STALE_AFTER
        {
            remove_if_present(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "temporary_tests.rs"]
mod tests;
