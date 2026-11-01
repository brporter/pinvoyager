//! Existing browser clients delete photos without native revision or receipt headers.
#[path = "support/native_client.rs"]
mod client;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use client::{Client, body, id};
use pinvoyager_api::{
    auth::session,
    photos::storage::{self, ContentHash},
};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

#[sqlx::test]
async fn browser_photo_deletion_enforces_ownership_and_defers_file_cleanup(pool: PgPool) {
    let owner = Client::new(pool.clone()).await;
    let stranger = Client::new(pool).await;
    let voyage = owner.create().await;
    let upload = owner
        .upload(id(&voyage), Uuid::now_v7(), Some("draft"), &client::jpeg())
        .await;
    assert_eq!(upload.status(), StatusCode::CREATED);
    let upload = body(upload).await;
    let photo = id(&upload["photos"][0]);
    let hash: String = sqlx::query_scalar("SELECT content_hash FROM photos WHERE id=$1")
        .bind(photo)
        .fetch_one(&owner.pool)
        .await
        .expect("photo hash");
    let original = storage::original_path(
        &owner.root,
        &ContentHash::try_from(hash.clone()).expect("hash"),
    );
    let owner_cookie = session::create(&owner.pool, owner.user_id, &owner.config.auth.session)
        .await
        .expect("owner browser session");
    let stranger_cookie =
        session::create(&owner.pool, stranger.user_id, &owner.config.auth.session)
            .await
            .expect("stranger browser session");
    for (token, expected) in [
        (stranger_cookie.expose(), StatusCode::NOT_FOUND),
        (owner_cookie.expose(), StatusCode::NO_CONTENT),
        (owner_cookie.expose(), StatusCode::NOT_FOUND),
    ] {
        let response = owner
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/photos/{photo}"))
                    .header("cookie", format!("pv_session={token}"))
                    .body(Body::empty())
                    .expect("legacy deletion"),
            )
            .await
            .expect("router");
        assert_eq!(response.status(), expected);
    }
    let photos: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE id=$1")
        .bind(photo)
        .fetch_one(&owner.pool)
        .await
        .expect("deleted photo");
    assert_eq!(photos, 0);
    let queued: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM photo_cleanup WHERE content_hash=$1)")
            .bind(hash)
            .fetch_one(&owner.pool)
            .await
            .expect("durable cleanup");
    assert!(queued);
    assert!(
        original.is_file(),
        "HTTP deletion leaves file removal to maintenance"
    );
    assert_eq!(
        owner
            .request(
                "GET",
                &format!("/api/voyages/{}", id(&voyage)),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::OK
    );
}
