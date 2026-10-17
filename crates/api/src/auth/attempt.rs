//! Pending provider authorization attempts.
//!
//! Starting authorization stores an attempt. A callback atomically deletes and
//! returns it, so only one callback can consume a given state.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::TryRng as _;
use rand::rngs::SysRng;
use sha2::{Digest as _, Sha256};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::auth::ProviderKind;

/// Number of random bytes used for a browser-binding value.
const BINDING_BYTES: usize = 32;

/// Browser-binding value whose hash is stored with an authorization attempt.
///
/// Uses a separate `SameSite=None` cookie so Apple's cross-site POST callback
/// can present it even when the session cookie is withheld.
pub struct BindingValue(String);

impl std::fmt::Debug for BindingValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BindingValue(redacted)")
    }
}

impl BindingValue {
    pub fn generate() -> Self {
        let mut bytes = [0u8; BINDING_BYTES];
        SysRng
            .try_fill_bytes(&mut bytes)
            .expect("OS CSPRNG unavailable; refusing to start an authorization attempt");
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn from_cookie_value(value: &str) -> Self {
        Self(value.to_owned())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> Vec<u8> {
        Sha256::digest(self.0.as_bytes()).to_vec()
    }

    /// Compares this value's SHA-256 hash with the stored hash in constant time.
    pub fn matches(&self, stored_hash: &[u8]) -> bool {
        let computed = self.hash();
        if computed.len() != stored_hash.len() {
            return false;
        }
        computed
            .iter()
            .zip(stored_hash)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

#[derive(Debug, Clone)]
pub struct PendingAttempt {
    pub state: String,
    pub provider: ProviderKind,
    /// Must match the ID token's `nonce` claim. Replay protection.
    pub nonce: String,
    /// PKCE verifier, sent at the code exchange.
    pub pkce_verifier: String,
    /// `Some` when this flow is "link an additional provider to this account"
    /// rather than a sign-in. D3 requires the linking user to be known from the
    /// authenticated session that *started* the flow — never inferred from the
    /// identity that comes back.
    pub link_user_id: Option<Uuid>,
    pub redirect_to: Option<String>,
    pub expires_at: DateTime<Utc>,
    /// SHA-256 of the browser-binding cookie. The callback must present the
    /// matching value to complete the attempt in the browser that started it.
    pub binding_hash: Vec<u8>,
}

/// Records a pending attempt with a short TTL.
pub async fn create(
    executor: impl PgExecutor<'_>,
    attempt: &PendingAttempt,
) -> Result<(), sqlx::Error> {
    // Store the provider's lowercase protocol name in the text column.
    sqlx::query!(
        "INSERT INTO auth_attempts
             (state, provider, nonce, pkce_verifier, link_user_id, redirect_to, expires_at,
              binding_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        attempt.state,
        attempt.provider.as_str(),
        attempt.nonce,
        attempt.pkce_verifier,
        attempt.link_user_id,
        attempt.redirect_to.as_deref(),
        attempt.expires_at,
        attempt.binding_hash,
    )
    .execute(executor)
    .await?;

    Ok(())
}

/// Atomically deletes and returns an unexpired attempt matching the state.
///
/// Returns `Ok(None)` for missing, consumed, expired, or unrecognized-provider
/// attempts. Expired attempts are deleted. Database failures return an error.
pub async fn consume(
    executor: impl PgExecutor<'_>,
    state: &str,
) -> Result<Option<PendingAttempt>, sqlx::Error> {
    // DELETE RETURNING lets only one concurrent callback consume the state.
    let row = sqlx::query!(
        "DELETE FROM auth_attempts
          WHERE state = $1
         RETURNING state, provider, nonce, pkce_verifier, link_user_id, redirect_to, expires_at,
                   binding_hash",
        state,
    )
    .fetch_optional(executor)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    if row.expires_at <= Utc::now() {
        return Ok(None);
    }

    let Some(provider) = ProviderKind::parse(&row.provider) else {
        // Treat an unrecognized stored provider as an unusable attempt.
        tracing::error!(provider = %row.provider, "auth attempt names an unknown provider");
        return Ok(None);
    };

    Ok(Some(PendingAttempt {
        state: row.state,
        provider,
        nonce: row.nonce,
        pkce_verifier: row.pkce_verifier,
        link_user_id: row.link_user_id,
        redirect_to: row.redirect_to,
        expires_at: row.expires_at,
        binding_hash: row.binding_hash,
    }))
}

/// Deletes attempts past their TTL. Returns how many.
pub async fn sweep_expired(executor: impl PgExecutor<'_>) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM auth_attempts WHERE expires_at <= now()")
        .execute(executor)
        .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
#[path = "attempt_tests.rs"]
mod tests;
