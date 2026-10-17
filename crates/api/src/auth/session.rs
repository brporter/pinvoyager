//! Server-side sessions identified by hashed bearer tokens.
//!
//! Each valid lookup updates activity for the idle timeout. The absolute
//! expiration remains fixed, and deleting the row revokes the session.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::TryRng as _;
use rand::rngs::SysRng;
use sha2::{Digest as _, Sha256};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::config::SessionConfig;

/// Number of random bytes used to generate a session token.
const TOKEN_BYTES: usize = 32;

/// Raw bearer token used in the session cookie. Only its hash is persisted.
/// `Debug` redacts the value; reading it requires `expose`.
pub struct SessionToken(String);

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(redacted)")
    }
}

impl SessionToken {
    /// Generates a token from 32 OS-random bytes. Panics if the OS random source fails.
    pub fn generate() -> Self {
        let mut bytes = [0u8; TOKEN_BYTES];

        // Fail token generation if the OS random source is unavailable.
        SysRng
            .try_fill_bytes(&mut bytes)
            .expect("OS CSPRNG unavailable; refusing to issue a session token");

        // URL-safe and unpadded so the value needs no cookie quoting or percent
        // encoding on the way out or back.
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Reconstructs from a cookie value. Does not validate — lookup does that.
    pub fn from_cookie_value(value: &str) -> Self {
        Self(value.to_owned())
    }

    /// The value to put in the cookie.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// SHA-256 of the token. This is the database key.
    pub fn hash(&self) -> Vec<u8> {
        // Hash the random bearer token so the database stores no usable raw token.
        Sha256::digest(self.0.as_bytes()).to_vec()
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// Converts seconds to the SQL parameter type, saturating at `i64::MAX`.
fn secs(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Issues a session for a user and returns the raw token for the cookie.
pub async fn create(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    config: &SessionConfig,
) -> Result<SessionToken, sqlx::Error> {
    let token = SessionToken::generate();

    // Use the database clock for both expiration creation and later comparisons.
    sqlx::query!(
        "INSERT INTO sessions (token_hash, user_id, absolute_expires_at)
         VALUES ($1, $2, now() + ($3::bigint * interval '1 second'))",
        token.hash(),
        user_id,
        secs(config.absolute_timeout_secs),
    )
    .execute(executor)
    .await?;

    Ok(token)
}

/// Looks up a token and updates `last_seen_at` if both timeouts allow it.
///
/// Deletes expired matches and returns `Ok(None)` for missing or expired sessions.
/// The absolute expiration is unchanged. Database failures return an error.
pub async fn lookup_and_touch(
    executor: impl PgExecutor<'_>,
    token: &SessionToken,
    config: &SessionConfig,
) -> Result<Option<Session>, sqlx::Error> {
    let idle = secs(config.idle_timeout_secs);

    // The DELETE and UPDATE use complementary expiry conditions and the same
    // transaction timestamp, so they cannot modify the same row.
    sqlx::query_as!(
        Session,
        "WITH expired AS (
             DELETE FROM sessions
              WHERE token_hash = $1
                AND (absolute_expires_at <= now()
                     OR last_seen_at <= now() - ($2::bigint * interval '1 second'))
         )
         UPDATE sessions
            SET last_seen_at = now()
          WHERE token_hash = $1
            AND absolute_expires_at > now()
            AND last_seen_at > now() - ($2::bigint * interval '1 second')
         RETURNING user_id, created_at, last_seen_at, absolute_expires_at",
        token.hash(),
        idle,
    )
    .fetch_optional(executor)
    .await
}

/// Deletes the session row. Missing sessions succeed; database failures return
/// an error.
pub async fn revoke(
    executor: impl PgExecutor<'_>,
    token: &SessionToken,
) -> Result<(), sqlx::Error> {
    // Deleting nothing is success: logging out of a session that has already
    // expired or been revoked elsewhere is not an error the caller can act on.
    sqlx::query!("DELETE FROM sessions WHERE token_hash = $1", token.hash())
        .execute(executor)
        .await?;

    Ok(())
}

/// Revokes every active session for the account.
pub async fn revoke_all_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM sessions WHERE user_id = $1", user_id)
        .execute(executor)
        .await?;

    Ok(result.rows_affected())
}

/// Deletes rows past their absolute ceiling. Returns how many.
pub async fn sweep_expired(executor: impl PgExecutor<'_>) -> Result<u64, sqlx::Error> {
    // This sweep checks absolute expiry only. Idle-expired sessions are rejected
    // and deleted by lookup, or removed here once their absolute expiry passes.
    let result = sqlx::query!("DELETE FROM sessions WHERE absolute_expires_at <= now()")
        .execute(executor)
        .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
