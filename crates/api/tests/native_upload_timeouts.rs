#[path = "support/native_client.rs"]
mod client;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use client::{Client, id};
use pinvoyager_api::photos::worker::VariantJob;
use sqlx::PgPool;
use std::time::Duration;
use tokio_stream::StreamExt as _;
use tower::ServiceExt as _;
use uuid::Uuid;

#[sqlx::test]
async fn slow_upload_has_its_own_timeout_but_other_requests_keep_the_short_limit(pool: PgPool) {
    let client = Client::configured(pool, |config| {
        config.server.request_timeout_secs = 1;
        config.server.upload_request_timeout_secs = 4;
    })
    .await;
    let voyage = client.create().await;
    let chunks=vec![b"--slow\r\nContent-Disposition: form-data; name=\"file\"; filename=\"p.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n".to_vec(),client::jpeg(),b"\r\n--slow--\r\n".to_vec()];
    let stream = tokio_stream::iter(chunks).then(|chunk| async {
        tokio::time::sleep(Duration::from_millis(550)).await;
        Ok::<_, std::io::Error>(chunk)
    });
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/voyages/{}/photos", id(&voyage)))
        .header("authorization", format!("Bearer {}", client.token))
        .header("idempotency-key", Uuid::now_v7().to_string())
        .header("x-pinvoyager-upload-visibility", "draft")
        .header("content-type", "multipart/form-data; boundary=slow")
        .body(Body::from_stream(stream))
        .expect("request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::CREATED
    );
    let stream = tokio_stream::once(0).then(|_| async {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        Ok::<_, std::io::Error>(b"{\"display_name\":\"later-name\"}".to_vec())
    });
    let request = Request::builder()
        .method("PATCH")
        .uri("/api/me")
        .header("authorization", format!("Bearer {}", client.token))
        .header("content-type", "application/json")
        .body(Body::from_stream(stream))
        .expect("request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::REQUEST_TIMEOUT
    );
}

#[sqlx::test]
async fn a_full_conversion_queue_returns_retryable_failure_without_committing_a_photo(
    pool: PgPool,
) {
    let client = Client::configured(pool, |config| {
        config.photos.variant_queue_wait_secs = 1;
        config.server.upload_request_timeout_secs = 4;
    })
    .await;
    let voyage = client.create().await;
    for _ in 0..64 {
        client
            .variants
            .enqueue(VariantJob::new(Uuid::now_v7()))
            .await
            .expect("fill queue");
    }
    let result = client
        .upload(id(&voyage), Uuid::now_v7(), Some("draft"), &client::jpeg())
        .await;
    assert_eq!(result.status(), StatusCode::SERVICE_UNAVAILABLE);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id=$1")
        .bind(id(&voyage))
        .fetch_one(&client.pool)
        .await
        .expect("photos");
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn upload_inactivity_times_out_between_chunks_without_committing(pool: PgPool) {
    let client = Client::configured(pool, |config| {
        config.server.upload_request_timeout_secs = 4;
        config.server.upload_body_idle_timeout_secs = 1;
    })
    .await;
    let voyage = client.create().await;
    let chunks = vec![
        (
            0,
            b"--idle\r\nContent-Disposition: form-data; name=\"file\"; filename=\"p.jpg\"\r\n\r\n"
                .to_vec(),
        ),
        (1500, client::jpeg()),
    ];
    let stream = tokio_stream::iter(chunks).then(|(delay, chunk)| async move {
        tokio::time::sleep(Duration::from_millis(delay)).await;
        Ok::<_, std::io::Error>(chunk)
    });
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/voyages/{}/photos", id(&voyage)))
        .header("authorization", format!("Bearer {}", client.token))
        .header("idempotency-key", Uuid::now_v7().to_string())
        .header("x-pinvoyager-upload-visibility", "draft")
        .header("content-type", "multipart/form-data; boundary=idle")
        .body(Body::from_stream(stream))
        .expect("request");
    assert_eq!(
        client
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::REQUEST_TIMEOUT
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos")
        .fetch_one(&client.pool)
        .await
        .expect("photos");
    assert_eq!(count, 0);
}
