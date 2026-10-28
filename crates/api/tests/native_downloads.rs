#[path = "support/native_client.rs"]
mod client;
use axum::http::StatusCode;
use client::{Client, body, id};
use pinvoyager_api::photos::worker::{self, VariantJob};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test]
async fn only_the_owner_may_download_verified_stripped_offline_copies(pool: PgPool) {
    let owner = Client::new(pool.clone()).await;
    let stranger = Client::new(pool).await;
    let voyage = owner.create().await;
    let voyage = id(&voyage);
    let accepted = body(
        owner
            .upload(voyage, Uuid::now_v7(), Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo: Uuid = accepted["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    let path = format!("/api/voyages/{voyage}/offline-manifest");
    assert_eq!(
        owner.request("GET", &path, None, None, None).await.status(),
        StatusCode::CONFLICT
    );
    worker::process(
        &owner.pool,
        &owner.config,
        &VariantJob::new(photo).with_voyage(voyage),
    )
    .await
    .expect("convert");
    let response = owner.request("GET", &path, None, None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let manifest = body(response).await;
    assert_eq!(manifest["retention_permission"], "owner_offline_copy_v1");
    assert_eq!(manifest["owner_id"], owner.user_id.to_string());
    assert!(!manifest.to_string().contains("original"));
    for variant in manifest["photos"][0]["variants"]
        .as_array()
        .expect("variants")
    {
        let url = variant["url"].as_str().expect("url");
        assert_eq!(
            stranger
                .request("GET", url, None, None, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let response = owner.request("GET", url, None, None, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/webp");
        assert_eq!(
            response.headers()["x-pinvoyager-retention"],
            "owner_offline_copy_v1"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
            .await
            .expect("bytes");
        assert_eq!(variant["sha256"], hex::encode(Sha256::digest(&bytes)));
        assert_eq!(variant["byte_count"], bytes.len());
        assert!(!bytes.windows(4).any(|chunk| chunk == b"EXIF"));
    }
    assert_eq!(
        stranger
            .request("GET", &path, None, None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        owner
            .request(
                "GET",
                &format!("/api/photos/{photo}/offline/original"),
                None,
                None,
                None
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn unavailable_variant_files_return_private_errors_and_never_stream_directories(
    pool: PgPool,
) {
    use pinvoyager_api::photos::{
        VariantKind,
        storage::{self, ContentHash},
    };
    let owner = Client::new(pool).await;
    let voyage = id(&owner.create().await);
    let bytes = client::jpeg();
    let uploaded = body(
        owner
            .upload(voyage, Uuid::now_v7(), Some("draft"), &bytes)
            .await,
    )
    .await;
    let photo: Uuid = uploaded["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    worker::process(
        &owner.pool,
        &owner.config,
        &VariantJob::new(photo).with_voyage(voyage),
    )
    .await
    .expect("conversion");
    let path = storage::variant_path(
        &owner.config.photos.storage_root,
        &ContentHash::try_from(hex::encode(Sha256::digest(&bytes))).expect("hash"),
        VariantKind::Thumb,
    );
    let original = std::fs::read(&path).expect("variant");
    std::fs::remove_file(&path).expect("remove missing file");
    let url = format!("/api/photos/{photo}/offline/thumb");
    let manifest = format!("/api/voyages/{voyage}/offline-manifest");
    assert_eq!(
        owner.request("GET", &url, None, None, None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        owner
            .request("GET", &manifest, None, None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    std::fs::create_dir(&path).expect("corrupt storage directory");
    assert_eq!(
        owner.request("GET", &url, None, None, None).await.status(),
        StatusCode::NOT_FOUND
    );
    let response = owner.request("GET", &manifest, None, None, None).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        !body(response)
            .await
            .to_string()
            .contains(&owner.root.to_string_lossy().to_string())
    );
    std::fs::remove_dir(&path).expect("remove directory");
    std::fs::write(&path, original).expect("restore variant");
    assert_eq!(
        owner
            .request("GET", &manifest, None, None, None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn ordinary_image_validators_never_hide_missing_or_unreadable_variant_files(pool: PgPool) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt as _;
    let owner = Client::new(pool).await;
    let voyage = id(&owner.create().await);
    let uploaded = body(
        owner
            .upload(voyage, Uuid::now_v7(), Some("draft"), &client::jpeg())
            .await,
    )
    .await;
    let photo: Uuid = uploaded["photos"][0]["id"]
        .as_str()
        .expect("photo")
        .parse()
        .expect("UUID");
    worker::process(
        &owner.pool,
        &owner.config,
        &VariantJob::new(photo).with_voyage(voyage),
    )
    .await
    .expect("ready photo");
    let path = format!("/api/photos/{photo}/thumb");
    let response = owner.request("GET", &path, None, None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let etag = response.headers()["etag"].clone();
    let hash: String = sqlx::query_scalar("SELECT content_hash FROM photos WHERE id=$1")
        .bind(photo)
        .fetch_one(&owner.pool)
        .await
        .expect("hash");
    let hash =
        pinvoyager_api::photos::storage::ContentHash::try_from(hash).expect("canonical hash");
    let thumbnail = pinvoyager_api::photos::storage::variant_path(
        &owner.root,
        &hash,
        pinvoyager_api::photos::VariantKind::Thumb,
    );
    std::fs::remove_file(&thumbnail).expect("simulate missing variant");
    let request = Request::builder()
        .uri(&path)
        .header("authorization", format!("Bearer {}", owner.token))
        .header("if-none-match", etag)
        .body(Body::empty())
        .expect("conditional image request");
    assert_eq!(
        owner
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router")
            .status(),
        StatusCode::NOT_FOUND
    );
    let variants = owner.root.join("variants");
    let backup = owner.root.join("variants-backup");
    std::fs::rename(&variants, &backup).expect("move variants");
    std::fs::write(&variants, b"obstructed image storage")
        .expect("simulate unreadable variants tree");
    let response = owner
        .request(
            "GET",
            &format!("/api/photos/{photo}/display"),
            None,
            None,
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body(response).await.to_string().contains("obstructed"));
    std::fs::remove_file(&variants).expect("remove obstruction");
    std::fs::rename(backup, variants).expect("restore variants");
}
