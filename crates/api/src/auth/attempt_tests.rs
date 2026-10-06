use super::*;

use sqlx::{PgPool, postgres::PgPoolOptions};

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

#[test]
fn a_binding_value_rejects_a_hash_with_the_wrong_length() {
    let value = BindingValue::from_cookie_value("cookie-value");
    assert!(!value.matches(&[0; 31]));
}

#[tokio::test]
async fn database_errors_are_returned_to_the_caller() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(100))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("valid URL creates a lazy pool");

    assert!(create(&pool, &attempt("unavailable-create")).await.is_err());
    assert!(consume(&pool, "unavailable-consume").await.is_err());
    assert!(sweep_expired(&pool).await.is_err());
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

#[sqlx::test]
async fn a_corrupt_provider_name_is_rejected_after_consuming_the_attempt(pool: PgPool) {
    sqlx::query("ALTER TABLE auth_attempts DROP CONSTRAINT auth_attempts_provider_check")
        .execute(&pool)
        .await
        .expect("remove provider check for this isolated test database");
    sqlx::query(
        "INSERT INTO auth_attempts
         (state, provider, nonce, pkce_verifier, link_user_id, redirect_to, expires_at, binding_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind("corrupt-provider")
    .bind("removed-provider")
    .bind("nonce")
    .bind("verifier")
    .bind(Option::<uuid::Uuid>::None)
    .bind(Option::<String>::None)
    .bind(Utc::now() + chrono::Duration::minutes(1))
    .bind(vec![0u8; 32])
    .execute(&pool)
    .await
    .expect("insert an otherwise valid corrupt row");

    assert!(
        consume(&pool, "corrupt-provider")
            .await
            .expect("consume corrupt row")
            .is_none()
    );
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
    let pending = attempt("state-expired");
    create(&pool, &pending).await.expect("create");
    sqlx::query(
        "UPDATE auth_attempts SET expires_at = now() - interval '1 second' WHERE state = $1",
    )
    .bind(&pending.state)
    .execute(&pool)
    .await
    .expect("expire attempt using the database clock");

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
    create(&pool, &attempt("state-stale"))
        .await
        .expect("create stale attempt");
    sqlx::query(
        "UPDATE auth_attempts SET expires_at = now() - interval '1 minute' WHERE state = $1",
    )
    .bind("state-stale")
    .execute(&pool)
    .await
    .expect("expire only the stale attempt using the database clock");

    assert_eq!(sweep_expired(&pool).await.expect("sweep"), 1);
    assert!(
        consume(&pool, "state-live")
            .await
            .expect("consume")
            .is_some()
    );
}
