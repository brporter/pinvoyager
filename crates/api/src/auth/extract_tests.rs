use super::*;

use axum::http::Request;
use secrecy::SecretString;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

use crate::auth::ProviderRegistry;
use crate::config::{AuthConfig, Config, DatabaseConfig};

fn app_state(pool: PgPool) -> AppState {
    let config = Config {
        server: crate::config::ServerConfig::default(),
        database: DatabaseConfig {
            url: SecretString::from("postgres://unused".to_owned()),
            max_connections: 1,
            min_connections: 1,
            acquire_timeout_secs: 5,
        },
        telemetry: crate::config::TelemetryConfig::default(),
        auth: AuthConfig {
            initial_admins: Default::default(),
            base_url: "https://example.test".parse().expect("valid base url"),
            session: crate::config::SessionConfig::default(),
            providers: crate::config::ProvidersConfig::default(),
            dev_login: false,
        },
        photos: crate::config::PhotosConfig::default(),
    };

    AppState::new(
        pool,
        std::sync::Arc::new(config),
        ProviderRegistry::empty(),
        crate::photos::worker::VariantQueue::detached(),
    )
}

fn parts(cookie: Option<&str>) -> Parts {
    let mut builder = Request::builder().uri("/api/me");
    if let Some(cookie) = cookie {
        builder = builder.header(axum::http::header::COOKIE, cookie);
    }

    builder.body(()).expect("valid request").into_parts().0
}

fn unavailable_pool() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("a lazy pool accepts a valid URL")
}

#[sqlx::test]
async fn current_user_resolves_from_the_session_cookie(pool: PgPool) {
    let state = app_state(pool.clone());
    let user = crate::db::repo::users::create(&pool, Some("Ada"), None)
        .await
        .expect("create user");

    let token = session::create(&pool, user.id, &state.config.auth.session)
        .await
        .expect("create session");

    let cookie = format!("pv_session={}", token.expose());
    let mut parts = parts(Some(&cookie));

    let current = CurrentUser::from_request_parts(&mut parts, &state)
        .await
        .expect("authenticated");

    assert_eq!(current.user_id, user.id);
    assert_eq!(current.session.user_id, user.id);
}

#[sqlx::test]
async fn extraction_slides_the_session_forward(pool: PgPool) {
    let state = app_state(pool.clone());
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create user");

    let token = session::create(&pool, user.id, &state.config.auth.session)
        .await
        .expect("create session");

    sqlx::query("UPDATE sessions SET last_seen_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .expect("age session");

    let cookie = format!("pv_session={}", token.expose());
    let mut parts = parts(Some(&cookie));

    // Optional auth is still activity, so this must slide the expiry too.
    let MaybeUser(user) = MaybeUser::from_request_parts(&mut parts, &state)
        .await
        .expect("infallible");
    assert!(user.is_some());

    let last_seen_is_recent: bool =
        sqlx::query_scalar("SELECT last_seen_at > now() - interval '1 minute' FROM sessions")
            .fetch_one(&pool)
            .await
            .expect("check session against the database clock");
    assert!(last_seen_is_recent, "session activity should slide forward");
}

#[sqlx::test]
async fn current_user_is_unauthorized_without_a_cookie(pool: PgPool) {
    let state = app_state(pool);
    let mut parts = parts(None);

    let err = CurrentUser::from_request_parts(&mut parts, &state)
        .await
        .expect_err("no session");

    assert!(matches!(err, AppError::Unauthorized));
}

#[sqlx::test]
async fn current_user_is_unauthorized_when_the_session_was_revoked(pool: PgPool) {
    let state = app_state(pool.clone());
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create user");

    let token = session::create(&pool, user.id, &state.config.auth.session)
        .await
        .expect("create session");
    session::revoke(&pool, &token).await.expect("revoke");

    let cookie = format!("pv_session={}", token.expose());
    let mut parts = parts(Some(&cookie));

    let err = CurrentUser::from_request_parts(&mut parts, &state)
        .await
        .expect_err("revoked session");

    assert!(matches!(err, AppError::Unauthorized));
}

#[sqlx::test]
async fn maybe_user_is_none_for_an_anonymous_caller(pool: PgPool) {
    let state = app_state(pool);
    let mut parts = parts(Some("some_other_cookie=value"));

    let MaybeUser(user) = MaybeUser::from_request_parts(&mut parts, &state)
        .await
        .expect("infallible");

    assert!(user.is_none());
}

#[tokio::test]
async fn maybe_user_degrades_to_anonymous_when_database_lookup_fails() {
    let pool = unavailable_pool();
    let state = app_state(pool);
    let mut parts = parts(Some("pv_session=unresolvable"));

    let MaybeUser(user) = MaybeUser::from_request_parts(&mut parts, &state)
        .await
        .expect("optional authentication is infallible");

    assert!(user.is_none());
}

#[tokio::test]
async fn current_user_propagates_database_lookup_failures() {
    let pool = unavailable_pool();
    let state = app_state(pool);
    let mut parts = parts(Some("pv_session=unresolvable"));

    let result = CurrentUser::from_request_parts(&mut parts, &state).await;

    assert!(result.is_err());
}
