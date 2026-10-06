//! Server-side sessions.
//!
//! CONTRACT — signatures below are fixed; implementations are owned by the
//! session agent.
//!
//! Sessions are server-side so they can be **revoked**. A self-contained signed
//! cookie cannot be invalidated before it expires, which matters more here than
//! in a typical application: with social-only sign-in the account *is* the
//! identity, and there is no password to change if something goes wrong.
//!
//! Expiry is sliding with an absolute ceiling. Sliding alone means a stolen
//! cookie lives forever provided it keeps being used.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::TryRng as _;
use rand::rngs::SysRng;
use sha2::{Digest as _, Sha256};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::config::SessionConfig;

/// 256 bits. The token is a bearer credential with no structure to guess at, so
/// entropy is the whole of its security; 256 bits puts brute force beyond
/// reach and costs 43 characters in a cookie.
const TOKEN_BYTES: usize = 32;

/// A raw session token. Exists in exactly two places: this type, and the user's
/// cookie. **Never** written to the database — only its hash is.
///
/// Deliberately has no `Debug` that reveals its contents and no `Display`, so
/// it cannot reach a log line by accident.
pub struct SessionToken(String);

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(redacted)")
    }
}

impl SessionToken {
    /// Generates a new token with at least 256 bits of entropy from the OS CSPRNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; TOKEN_BYTES];

        // `SysRng` is the operating system's generator, not a userspace PRNG
        // seeded from it. It can only fail if the OS entropy source is
        // unavailable, and the sole alternative to panicking there would be to
        // issue a session token of unknown quality — which is worse than not
        // issuing one at all.
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
        // A plain digest, deliberately: unlike a password this is 256 bits of
        // uniform randomness, so there is no dictionary to stretch against and
        // a KDF here would only add per-request cost. What the hash buys is
        // that a database leak yields no usable session.
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

/// Timeouts are configured as `u64` seconds but bind as `bigint`. A value large
/// enough to saturate here is already far past what Postgres can hold in an
/// `interval`, so it fails loudly at query time rather than silently wrapping
/// into a short — or negative — timeout.
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

    // The ceiling is computed by the database rather than in Rust so that it is
    // on the same clock as every later comparison against it. Anything else
    // makes session lifetime depend on the drift between app host and database.
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

/// Looks a session up and, if valid, slides its expiry forward.
///
/// Returns `Ok(None)` for absent, idle-expired, or past-absolute-ceiling — the
/// caller cannot distinguish them and does not need to. An expired row should
/// be deleted on the way past rather than left to the sweeper.
///
/// Note this is called on every authenticated request, so it must be a single
/// round trip: do the expiry check and the touch in one statement rather than
/// a SELECT followed by an UPDATE.
pub async fn lookup_and_touch(
    executor: impl PgExecutor<'_>,
    token: &SessionToken,
    config: &SessionConfig,
) -> Result<Option<Session>, sqlx::Error> {
    let idle = secs(config.idle_timeout_secs);

    // One statement, one round trip, on every authenticated request.
    //
    // The two arms are exact complements: `now()` is the transaction timestamp
    // and so is a single instant for the whole statement, which means the
    // deleting CTE and the touching UPDATE can never match the same row. That
    // is what makes combining two data-modifying statements safe here — the
    // "unpredictable results" warning in the Postgres manual is about two
    // sub-statements racing to modify one row, which these cannot do.
    //
    // Both conditions must hold for a session to be live: the sliding idle
    // window *and* the absolute ceiling. The ceiling is never extended.
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

/// Destroys one session. Used by logout — which must delete the row, not
/// merely clear the cookie.
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

/// Destroys every session for a user. Phase 8's account deletion needs this,
/// and it is the mechanism behind any future "sign me out everywhere".
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
    // Only the ceiling, because that is the one bound that does not depend on
    // configuration: the idle window is a `SessionConfig` value this signature
    // does not receive, and an idle-expired row is already unusable — the next
    // lookup deletes it, and this sweep collects it at the ceiling regardless.
    let result = sqlx::query!("DELETE FROM sessions WHERE absolute_expires_at <= now()")
        .execute(executor)
        .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
