//! Retryable cleanup after photo rows have been committed as deleted.
use super::storage::{self, ContentHash};
use crate::{db::repo::photos, error::AppResult};
use sqlx::{PgConnection, PgPool};
use std::path::Path;

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
            storage::remove_content(&root, &content)
                .map_err(|error| crate::AppError::Internal(Box::new(error)))?;
            Ok(tx)
        })
        .await?;
    }
    sqlx::query("DELETE FROM photo_cleanup WHERE content_hash=$1")
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Each maintenance pass is bounded; later passes retry failures and drain
/// any remaining records without delaying HTTP startup.
pub async fn sweep(pool: &PgPool, root: &Path) -> AppResult<()> {
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT content_hash FROM photo_cleanup ORDER BY created_at LIMIT 1000")
            .fetch_all(pool)
            .await?;
    for hash in hashes {
        if let Err(error) = clean(pool, root, &hash).await {
            tracing::warn!(%error, "photo cleanup will be retried");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
