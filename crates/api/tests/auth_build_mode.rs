//! Verify the HTTP bypass is unavailable when disabled or compiled for release.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use pinvoyager_api::{
    AppState, Config, auth::ProviderRegistry, http, photos::worker::VariantQueue,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt as _;

async fn assert_development_route_absent(enabled: bool) {
    let mut config = Config::load().expect("config");
    config.auth.dev_login = enabled;
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("lazy pool");
    let app = http::router(AppState::new(
        pool,
        Arc::new(config),
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/dev/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"subject":"admin@example.test","grant_admin":true}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!response.headers().contains_key("set-cookie"));
}

#[tokio::test]
async fn disabled_development_login_returns_404() {
    assert_development_route_absent(false).await;
}

#[cfg(not(debug_assertions))]
#[tokio::test]
async fn release_build_omits_development_login_even_when_enabled() {
    assert_development_route_absent(true).await;
}
