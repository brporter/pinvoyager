//! Account removal and durable image cleanup. Callers lock the account first.
use crate::{AppResult, db::repo::photos, photos::cleanup};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub async fn remove(connection: &mut PgConnection, account: Uuid) -> AppResult<Vec<String>> {
    sqlx::query("SELECT id FROM voyages WHERE owner_id=$1 ORDER BY id FOR UPDATE")
        .bind(account)
        .fetch_all(&mut *connection)
        .await?;
    let hashes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT content_hash FROM photos WHERE owner_id=$1 ORDER BY content_hash",
    )
    .bind(account)
    .fetch_all(&mut *connection)
    .await?;
    for hash in &hashes {
        photos::lock_content(connection, hash).await?;
    }
    // Profile-name aliases remain reserved by their ON DELETE SET NULL rule.
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(account)
        .execute(&mut *connection)
        .await?;
    for hash in &hashes {
        cleanup::schedule(connection, hash).await?;
    }
    Ok(hashes)
}
pub async fn sweep(pool: &PgPool) -> AppResult<()> {
    // File cleanup owns retries and reference checks. Its committed records are
    // the completion evidence, so one failing file cannot delay other accounts.
    sqlx::query("UPDATE account_deletions d SET state='complete',hashes='{}',account_id=NULL WHERE state='deleting' AND NOT EXISTS(SELECT 1 FROM photo_cleanup c WHERE c.content_hash=ANY(d.hashes))").execute(pool).await?;
    sqlx::query("DELETE FROM deletion_proofs WHERE expires_at<=now()")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM account_deletions WHERE retain_until<=now() AND state<>'deleting'")
        .execute(pool)
        .await?;
    Ok(())
}
