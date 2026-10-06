use super::*;

use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

fn config() -> SessionConfig {
    SessionConfig::default()
}

#[test]
fn generate_produces_a_urlsafe_token_of_at_least_256_bits() {
    let token = SessionToken::generate();
    let decoded = URL_SAFE_NO_PAD
        .decode(token.expose())
        .expect("token is URL-safe base64");

    assert!(
        decoded.len() >= 32,
        "want >= 256 bits, got {}",
        decoded.len()
    );
    assert!(
        token
            .expose()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "token must be safe in a cookie and a URL without escaping"
    );
}

#[test]
fn generate_does_not_repeat() {
    let a = SessionToken::generate();
    let b = SessionToken::generate();
    assert_ne!(a.expose(), b.expose());
}

#[test]
fn hash_is_sha256_of_the_token() {
    // Known answer, so that a future change of digest cannot pass silently:
    // every existing session would be invalidated by one.
    let token = SessionToken::from_cookie_value("abc");
    let hex: String = token.hash().iter().map(|b| format!("{b:02x}")).collect();

    assert_eq!(
        hex,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn hash_is_deterministic_and_distinguishes_tokens() {
    let a = SessionToken::from_cookie_value("one");
    let b = SessionToken::from_cookie_value("two");

    assert_eq!(a.hash(), SessionToken::from_cookie_value("one").hash());
    assert_ne!(a.hash(), b.hash());
    assert_eq!(a.hash().len(), 32);
}

#[test]
fn debug_does_not_reveal_the_token() {
    let token = SessionToken::from_cookie_value("super-secret-value");
    let rendered = format!("{token:?}");

    assert!(!rendered.contains("super-secret-value"));
}

async fn a_user(pool: &PgPool) -> Uuid {
    crate::db::repo::users::create(pool, Some("Test"), None)
        .await
        .expect("create user")
        .id
}

/// Ages a session's timestamps directly, so the expiry tests do not have to
/// sleep for a real timeout.
async fn age(pool: &PgPool, token: &SessionToken, last_seen: &str, absolute: &str) {
    sqlx::query(
        "UPDATE sessions
                SET last_seen_at = now() + $2::interval,
                    absolute_expires_at = now() + $3::interval
              WHERE token_hash = $1",
    )
    .bind(token.hash())
    .bind(last_seen)
    .bind(absolute)
    .execute(pool)
    .await
    .expect("age session");
}

#[sqlx::test]
async fn a_session_survives_lookup_and_slides_forward(pool: PgPool) {
    let user_id = a_user(&pool).await;
    let token = create(&pool, user_id, &config()).await.expect("create");

    // Start with a session last seen well inside the idle window, so that
    // the touch has something visible to move.
    age(&pool, &token, "-1 hour", "30 days").await;

    let session = lookup_and_touch(&pool, &token, &config())
        .await
        .expect("lookup")
        .expect("session is live");

    assert_eq!(session.user_id, user_id);
    let database_now: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&pool)
        .await
        .expect("read database clock");
    assert!(
        session.last_seen_at > database_now - chrono::Duration::minutes(1),
        "last_seen_at should have slid forward to now"
    );

    // And the ceiling is untouched by the slide.
    let after = lookup_and_touch(&pool, &token, &config())
        .await
        .expect("lookup")
        .expect("still live");
    assert_eq!(after.absolute_expires_at, session.absolute_expires_at);
}

#[sqlx::test]
async fn an_unknown_token_does_not_resolve(pool: PgPool) {
    let token = SessionToken::generate();

    assert!(
        lookup_and_touch(&pool, &token, &config())
            .await
            .expect("lookup")
            .is_none()
    );
}

#[sqlx::test]
async fn an_idle_expired_session_does_not_resolve(pool: PgPool) {
    let user_id = a_user(&pool).await;
    let token = create(&pool, user_id, &config()).await.expect("create");

    // Inside the ceiling, outside the idle window.
    age(&pool, &token, "-30 days", "30 days").await;

    assert!(
        lookup_and_touch(&pool, &token, &config())
            .await
            .expect("lookup")
            .is_none()
    );

    // And it was cleaned up on the way past rather than left to the sweep.
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(remaining, 0);
}

/// The property that makes the ceiling worth having: activity does not save
/// a session that has run out of absolute time.
#[sqlx::test]
async fn a_session_past_the_ceiling_does_not_resolve_even_when_just_used(pool: PgPool) {
    let user_id = a_user(&pool).await;
    let token = create(&pool, user_id, &config()).await.expect("create");

    age(&pool, &token, "-1 second", "-1 second").await;

    assert!(
        lookup_and_touch(&pool, &token, &config())
            .await
            .expect("lookup")
            .is_none()
    );
}

#[sqlx::test]
async fn revoke_ends_the_session_immediately(pool: PgPool) {
    let user_id = a_user(&pool).await;
    let token = create(&pool, user_id, &config()).await.expect("create");

    assert!(
        lookup_and_touch(&pool, &token, &config())
            .await
            .expect("lookup")
            .is_some()
    );

    revoke(&pool, &token).await.expect("revoke");

    assert!(
        lookup_and_touch(&pool, &token, &config())
            .await
            .expect("lookup")
            .is_none()
    );

    // Revoking again is not an error.
    revoke(&pool, &token).await.expect("revoke is idempotent");
}

#[sqlx::test]
async fn revoke_all_for_user_leaves_other_users_alone(pool: PgPool) {
    let user_id = a_user(&pool).await;
    let other_id = a_user(&pool).await;

    let one = create(&pool, user_id, &config()).await.expect("create");
    let two = create(&pool, user_id, &config()).await.expect("create");
    let theirs = create(&pool, other_id, &config()).await.expect("create");

    assert_eq!(
        revoke_all_for_user(&pool, user_id)
            .await
            .expect("revoke all"),
        2
    );

    for token in [&one, &two] {
        assert!(
            lookup_and_touch(&pool, token, &config())
                .await
                .expect("lookup")
                .is_none()
        );
    }
    assert!(
        lookup_and_touch(&pool, &theirs, &config())
            .await
            .expect("lookup")
            .is_some()
    );
}

#[sqlx::test]
async fn sweep_removes_only_sessions_past_the_ceiling(pool: PgPool) {
    let user_id = a_user(&pool).await;
    let live = create(&pool, user_id, &config()).await.expect("create");
    let dead = create(&pool, user_id, &config()).await.expect("create");

    age(&pool, &dead, "-1 minute", "-1 minute").await;

    assert_eq!(sweep_expired(&pool).await.expect("sweep"), 1);
    assert!(
        lookup_and_touch(&pool, &live, &config())
            .await
            .expect("lookup")
            .is_some()
    );
}

#[tokio::test]
async fn database_failures_are_returned_by_session_writes() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("lazy pool accepts a syntactically valid URL");
    let user_id = Uuid::new_v4();
    let token = SessionToken::from_cookie_value("not-a-real-session");

    assert!(create(&pool, user_id, &config()).await.is_err());
    assert!(revoke(&pool, &token).await.is_err());
    assert!(revoke_all_for_user(&pool, user_id).await.is_err());
    assert!(sweep_expired(&pool).await.is_err());
}
