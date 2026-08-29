//! In-flight authorization attempts.
//!
//! CONTRACT — signatures below are fixed; implementations are owned by the
//! session agent.
//!
//! An attempt is created at `/auth/{provider}/start` and **consumed** at the
//! callback. Single-use is the point: it is what makes a replayed callback
//! fail. Consumption must therefore be an atomic `DELETE ... RETURNING`, not a
//! read followed by a delete — two concurrent callbacks with the same state
//! must not both succeed.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::TryRng as _;
use rand::rngs::SysRng;
use sha2::{Digest as _, Sha256};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::auth::ProviderKind;

/// 256 bits, same reasoning as a session token: an opaque value whose only
/// security is that it cannot be guessed.
const BINDING_BYTES: usize = 32;

/// The raw browser-binding value. Lives in exactly two places: this type and
/// the cookie. Only its hash reaches the database.
///
/// Separate from the session cookie on purpose. Matching the callback's
/// *session* against `link_user_id` would look simpler and would break Apple
/// outright — its callback is a cross-site form POST, and a `SameSite=Lax`
/// cookie is not sent on one. This value is carried `SameSite=None` so it
/// survives that POST, which is safe precisely because it is single-use, opaque,
/// and confers nothing on its own: possessing it lets you complete one specific
/// authorization attempt you already started, and nothing else.
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

    /// Constant-time comparison against a stored hash.
    ///
    /// Timing on a 32-byte digest comparison is not a realistic attack here —
    /// the value is single-use and expires in minutes — but the cost of getting
    /// it right is one function call, and a short-circuiting `==` in
    /// authentication code is the kind of thing that gets copied somewhere it
    /// does matter.
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
    /// SHA-256 of the binding value handed to the browser as a cookie at
    /// `/auth/{provider}/start`.
    ///
    /// The callback must present a cookie hashing to this, which is what ties
    /// the flow to the browser that began it. Without it, an attacker can start
    /// a link flow on their own account and lure a victim through the
    /// provider's consent screen, attaching the victim's identity to the
    /// attacker's account — see the migration for the full sequence.
    pub binding_hash: Vec<u8>,
}

/// Records a pending attempt with a short TTL.
pub async fn create(
    executor: impl PgExecutor<'_>,
    attempt: &PendingAttempt,
) -> Result<(), sqlx::Error> {
    // `provider` is a text column with a CHECK constraint rather than a
    // Postgres enum (see the identities migration), so the protocol-layer
    // `ProviderKind` writes its own wire spelling.
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

/// Atomically consumes an attempt by state.
///
/// `Ok(None)` covers unknown, already-used, and expired alike — the caller must
/// not distinguish them to the client. Implement as a single
/// `DELETE ... WHERE state = $1 RETURNING ...`; a SELECT-then-DELETE lets two
/// concurrent callbacks both pass.
pub async fn consume(
    executor: impl PgExecutor<'_>,
    state: &str,
) -> Result<Option<PendingAttempt>, sqlx::Error> {
    // This DELETE is the single-use property. Postgres serializes the two
    // deleters of one row and only the first gets a RETURNING row, so a
    // replayed callback — or a genuinely concurrent one — finds nothing. A
    // SELECT followed by a DELETE would let both readers pass the check before
    // either deleted, which is precisely the replay this table exists to stop.
    //
    // The expiry is checked afterwards rather than in the WHERE clause so that
    // an expired attempt is removed by the same statement that rejects it.
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
        // Unreachable behind the CHECK constraint, so this is a corrupt row or
        // a provider removed from the code while attempts were in flight.
        // Either way the flow cannot continue, and "no such attempt" is the
        // truthful answer to give the caller.
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
mod tests {
    use super::*;

    use sqlx::PgPool;

    fn attempt(state: &str) -> PendingAttempt {
        PendingAttempt {
            state: state.to_owned(),
            provider: ProviderKind::Google,
            nonce: "nonce-value".to_owned(),
            pkce_verifier: "verifier-value".to_owned(),
            link_user_id: None,
            redirect_to: Some("/voyages".to_owned()),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
            binding_hash: BindingValue::generate().hash(),
        }
    }

    /// The binding value must survive a cookie round trip and still match, and
    /// a different value must not — this is the whole of the link-CSRF defense,
    /// so it is asserted rather than assumed.
    #[test]
    fn a_binding_value_matches_only_its_own_hash() {
        let value = BindingValue::generate();
        let stored = value.hash();

        let presented = BindingValue::from_cookie_value(value.expose());
        assert!(presented.matches(&stored), "the same value must match");

        let other = BindingValue::generate();
        assert!(
            !other.matches(&stored),
            "a different value must not match — an attacker who cannot set a \
             cookie in the victim's browser is exactly this case"
        );

        assert!(
            !BindingValue::from_cookie_value("").matches(&stored),
            "an empty cookie value must not match"
        );
    }

    /// Two generated values must differ. A constant would make the cookie
    /// decorative while every test above still passed.
    #[test]
    fn binding_values_do_not_repeat() {
        let a = BindingValue::generate();
        let b = BindingValue::generate();
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn a_binding_value_does_not_leak_through_debug() {
        let value = BindingValue::generate();
        let rendered = format!("{value:?}");
        assert!(!rendered.contains(value.expose()));
    }

    #[sqlx::test]
    async fn an_attempt_round_trips(pool: PgPool) {
        let user = crate::db::repo::users::create(&pool, None, None)
            .await
            .expect("create user");

        let mut pending = attempt("state-round-trip");
        pending.provider = ProviderKind::Apple;
        pending.link_user_id = Some(user.id);

        create(&pool, &pending).await.expect("create");

        let consumed = consume(&pool, "state-round-trip")
            .await
            .expect("consume")
            .expect("attempt exists");

        assert_eq!(consumed.state, pending.state);
        assert_eq!(consumed.provider, ProviderKind::Apple);
        assert_eq!(consumed.nonce, pending.nonce);
        assert_eq!(consumed.pkce_verifier, pending.pkce_verifier);
        assert_eq!(consumed.link_user_id, Some(user.id));
        assert_eq!(consumed.redirect_to.as_deref(), Some("/voyages"));
    }

    /// Single-use, which is what makes replaying a callback fail.
    #[sqlx::test]
    async fn consuming_twice_yields_none_the_second_time(pool: PgPool) {
        create(&pool, &attempt("state-replay"))
            .await
            .expect("create");

        assert!(
            consume(&pool, "state-replay")
                .await
                .expect("consume")
                .is_some()
        );
        assert!(
            consume(&pool, "state-replay")
                .await
                .expect("consume")
                .is_none()
        );
    }

    /// The same property under genuine concurrency: the atomic DELETE is what
    /// stops two simultaneous callbacks with one state from both succeeding.
    #[sqlx::test]
    async fn concurrent_consumers_produce_exactly_one_winner(pool: PgPool) {
        create(&pool, &attempt("state-race")).await.expect("create");

        let a = pool.clone();
        let b = pool.clone();
        let (first, second) = tokio::join!(
            tokio::spawn(async move { consume(&a, "state-race").await }),
            tokio::spawn(async move { consume(&b, "state-race").await }),
        );

        let winners = [first, second]
            .into_iter()
            .filter(|r| matches!(r.as_ref().expect("task did not panic"), Ok(Some(_))))
            .count();

        assert_eq!(winners, 1, "exactly one caller may consume a state");
    }

    #[sqlx::test]
    async fn an_unknown_state_yields_none(pool: PgPool) {
        assert!(
            consume(&pool, "never-issued")
                .await
                .expect("consume")
                .is_none()
        );
    }

    #[sqlx::test]
    async fn an_expired_attempt_yields_none_and_is_removed(pool: PgPool) {
        let mut pending = attempt("state-expired");
        pending.expires_at = Utc::now() - chrono::Duration::seconds(1);
        create(&pool, &pending).await.expect("create");

        assert!(
            consume(&pool, "state-expired")
                .await
                .expect("consume")
                .is_none()
        );

        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_attempts")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(
            remaining, 0,
            "an expired attempt is consumed, not left behind"
        );
    }

    #[sqlx::test]
    async fn sweep_removes_only_expired_attempts(pool: PgPool) {
        create(&pool, &attempt("state-live")).await.expect("create");

        let mut stale = attempt("state-stale");
        stale.expires_at = Utc::now() - chrono::Duration::minutes(1);
        create(&pool, &stale).await.expect("create");

        assert_eq!(sweep_expired(&pool).await.expect("sweep"), 1);
        assert!(
            consume(&pool, "state-live")
                .await
                .expect("consume")
                .is_some()
        );
    }
}
