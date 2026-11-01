//! Native credentials must fail closed on public routes as well as account routes.
#[path = "support/native_client.rs"]
mod client;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use client::Client;
use pinvoyager_api::auth::session::SessionToken;
use sqlx::PgPool;
use tower::ServiceExt as _;

#[sqlx::test]
async fn expired_native_sessions_are_rejected_and_removed_without_anonymous_fallback(pool: PgPool) {
    for expiry in [
        "UPDATE sessions SET last_seen_at=now()-interval '15 days' WHERE user_id=$1",
        "UPDATE sessions SET absolute_expires_at=now()-interval '1 second' WHERE user_id=$1",
    ] {
        let client = Client::new(pool.clone()).await;
        sqlx::query(expiry)
            .bind(client.user_id)
            .execute(&pool)
            .await
            .expect("expire native session");
        for path in ["/api/voyages", "/api/me"] {
            assert_eq!(
                client.request("GET", path, None, None, None).await.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must reject an expired bearer credential"
            );
        }
        let remains: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=$1)")
                .bind(SessionToken::from_cookie_value(&client.token).hash())
                .fetch_one(&pool)
                .await
                .expect("session state");
        assert!(!remains, "expired sessions must not be revived by activity");
    }
}

#[sqlx::test]
async fn suspended_native_sessions_are_rejected_until_the_account_is_enabled(pool: PgPool) {
    let client = Client::new(pool).await;
    sqlx::query("UPDATE users SET suspended=true WHERE id=$1")
        .bind(client.user_id)
        .execute(&client.pool)
        .await
        .expect("suspend account without revoking its session");
    for path in ["/api/voyages", "/api/me"] {
        assert_eq!(
            client.request("GET", path, None, None, None).await.status(),
            StatusCode::UNAUTHORIZED,
            "{path} must recheck suspension for an existing session"
        );
    }
    sqlx::query("UPDATE users SET suspended=false WHERE id=$1")
        .bind(client.user_id)
        .execute(&client.pool)
        .await
        .expect("enable account");
    assert_eq!(
        client
            .request("GET", "/api/me", None, None, None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn malformed_native_credentials_never_become_anonymous_requests(pool: PgPool) {
    let client = Client::new(pool).await;
    for authorization in [
        "Basic credential".to_owned(),
        "Bearer ".to_owned(),
        "Bearer token extra".to_owned(),
        "Bearer token\tvalue".to_owned(),
        format!("Bearer {}", "a".repeat(1025)),
    ] {
        let response = client
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/voyages")
                    .header("authorization", authorization)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = client
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("authorization", format!("bEaReR {}", client.token))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the scheme is case insensitive"
    );
}
