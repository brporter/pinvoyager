//! Retryable deletion cleanup and recovery of old, unreferenced storage files.
use super::storage::{self, ContentHash};
use crate::{db::repo::photos, error::AppResult};
use sqlx::{PgConnection, PgPool};
use std::{path::Path, time::SystemTime};

/// Called inside the deletion transaction, after locking its content hashes.
pub async fn schedule(connection: &mut PgConnection, hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO photo_cleanup (content_hash) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(hash)
        .execute(connection)
        .await?;
    Ok(())
}

/// Upload, final variant writes, and cleanup serialize on the same hash.
/// Recheck references *after* acquiring the lock: another upload may have
/// adopted the content since deletion committed. No CPU work holds this lock.
pub async fn clean(pool: &PgPool, root: &Path, hash: &str) -> AppResult<()> {
    clean_content(pool, root, hash, None).await
}

/// With a cutoff, remove only older regular files and leave scheduled deletion
/// records alone. Without one, remove all content and complete its deletion record.
/// Both paths recheck database references under the upload's content lock.
pub(super) async fn clean_content(
    pool: &PgPool,
    root: &Path,
    hash: &str,
    cutoff: Option<SystemTime>,
) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    photos::lock_content(&mut tx, hash).await?;
    let referenced: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM photos WHERE content_hash=$1)")
            .bind(hash)
            .fetch_one(&mut *tx)
            .await?;
    if !referenced {
        let root = root.to_path_buf();
        let content = ContentHash::try_from(hash.to_owned())?;
        // A canceled caller must not release the lock while an unlink is still
        // running: a new upload could otherwise adopt and then lose this file.
        tx = crate::blocking::run_blocking(move || {
            if let Some(cutoff) = cutoff {
                super::orphans::remove_older_than(&root, &content, cutoff)
                    .map_err(|error| crate::AppError::Internal(Box::new(error)))?;
            } else {
                storage::remove_content(&root, &content)
                    .map_err(|error| crate::AppError::Internal(Box::new(error)))?;
            }
            Ok(tx)
        })
        .await?;
    }
    if cutoff.is_none() {
        sqlx::query("DELETE FROM photo_cleanup WHERE content_hash=$1")
            .bind(hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Sweeps temporary files, attempts at most 1,000 due deletion records, then
/// scans stored content for unreferenced files older than one day. Later passes
/// retry failures and drain remaining records. Database and scan errors propagate.
pub async fn sweep(pool: &PgPool, root: &Path) -> AppResult<()> {
    if let Err(error) = super::temporary::sweep(root).await {
        tracing::warn!(%error, "stale temporary upload cleanup will be retried");
    }
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT content_hash FROM photo_cleanup WHERE next_attempt_at <= now() ORDER BY next_attempt_at, created_at, content_hash LIMIT 1000")
            .fetch_all(pool)
            .await?;
    for hash in hashes {
        if let Err(error) = clean(pool, root, &hash).await {
            // Move failures behind currently due records, so a full page of
            // persistent failures cannot prevent newer cleanup from running.
            sqlx::query("UPDATE photo_cleanup SET next_attempt_at=now()+interval '60 seconds' WHERE content_hash=$1")
                .bind(&hash).execute(pool).await?;
            tracing::warn!(%error, "photo cleanup will be retried after 60 seconds");
        }
    }
    super::orphans::sweep(pool, root).await
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
