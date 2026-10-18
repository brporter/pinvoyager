use crate::photo_voyage;
use std::sync::Arc;
use std::time::Duration;

use axum::http::Request;
use secrecy::SecretString;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt as _;

use super::*;
use crate::config::{Config, DatabaseConfig, PhotosConfig, ServerConfig, TelemetryConfig};
use crate::photos::worker::STATUS_FAILED;

#[tokio::test]
async fn failed_upload_cleanup_logs_when_the_temporary_path_is_a_directory() {
    let scratch = crate::photos::decode::testutil::ScratchDir::new("upload-cleanup-error");
    let path = scratch.path().join("rejected-upload.part");
    std::fs::create_dir(&path).expect("create directory at the temporary file path");

    discard_failed_upload(&path).await;

    assert!(path.is_dir(), "cleanup does not remove a directory");
}

/// A state whose pool is lazy and never connects. Enough to prove routing
/// and every check that happens before a query.
fn test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("a syntactically valid url makes a lazy pool");
    test_state_with(
        pool,
        "data".into(),
        crate::photos::worker::VariantQueue::detached(),
    )
}

fn test_state_with_storage(storage_root: std::path::PathBuf) -> AppState {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("a syntactically valid url makes a lazy pool");
    test_state_with(
        pool,
        storage_root,
        crate::photos::worker::VariantQueue::detached(),
    )
}

fn test_state_with(
    pool: sqlx::PgPool,
    storage_root: std::path::PathBuf,
    queue: crate::photos::worker::VariantQueue,
) -> AppState {
    let config = Config {
        server: ServerConfig {
            bind: "127.0.0.1:0".parse().expect("valid socket addr"),
            shutdown_timeout_secs: 5,
            request_timeout_secs: 5,
        },
        database: DatabaseConfig {
            url: SecretString::from("postgres://nobody@127.0.0.1:1/none"),
            max_connections: 1,
            min_connections: 0,
            acquire_timeout_secs: 1,
        },
        telemetry: TelemetryConfig::default(),
        auth: crate::config::AuthConfig {
            base_url: "http://127.0.0.1:8080".parse().expect("valid url"),
            session: Default::default(),
            providers: Default::default(),
            dev_login: false,
        },
        photos: PhotosConfig {
            storage_root,
            ..PhotosConfig::default()
        },
    };

    AppState::new(
        pool,
        Arc::new(config),
        crate::auth::ProviderRegistry::empty(),
        queue,
    )
}

fn upload_test_state() -> (AppState, crate::photos::decode::testutil::ScratchDir) {
    let scratch = crate::photos::decode::testutil::ScratchDir::new("http-upload");
    (
        test_state_with_storage(scratch.path().to_path_buf()),
        scratch,
    )
}

fn test_user(user_id: Uuid) -> CurrentUser {
    let now = chrono::Utc::now();
    CurrentUser {
        user_id,
        session: crate::auth::session::Session {
            user_id,
            created_at: now,
            last_seen_at: now,
            absolute_expires_at: now + chrono::Duration::hours(1),
        },
    }
}

async fn create_photo_for_test(
    pool: &PgPool,
    owner_id: Uuid,
    content_hash: &str,
) -> crate::db::repo::photos::Photo {
    crate::db::repo::photos::create(
        pool,
        &crate::db::repo::photos::NewPhoto {
            voyage_id: {
                sqlx::query("INSERT INTO voyages (id, owner_id, title) VALUES ($1, $1, 'Photo fixture') ON CONFLICT DO NOTHING").bind(owner_id).execute(pool).await.expect("fixture voyage");
                owner_id
            },
            owner_id,
            content_hash: content_hash.to_owned(),
            byte_size: 123,
            width: 32,
            height: 24,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: None,
            longitude: None,
        },
    )
    .await
    .expect("insert test photo")
}

async fn status_of(method: &str, uri: &str) -> StatusCode {
    crate::http::router(test_state())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("router is infallible")
        .status()
}

/// The routes exist and every one of them requires a session. 401 rather
/// than 404 is also what proves they are merged into the router at all — an
/// unwired route would 404 here, with no database involved either way.
#[tokio::test]
async fn every_metadata_route_requires_a_session() {
    let id = Uuid::now_v7();

    // Image delivery permits anonymous access to published voyage variants.
    // Metadata routes require a session and ownership because they return exact GPS.
    for (method, uri) in [
        ("GET", format!("/api/voyages/{id}/photos")),
        ("POST", format!("/api/voyages/{id}/photos")),
        ("GET", format!("/api/photos/{id}")),
        ("DELETE", format!("/api/photos/{id}")),
    ] {
        assert_eq!(
            status_of(method, &uri).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn upload_rejects_a_declared_size_over_the_limit_before_reading() {
    let (state, _scratch) = upload_test_state();
    let declared = state.config.photos.max_upload_bytes + 1;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_LENGTH,
        declared.to_string().parse().expect("valid test fixture"),
    );
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("valid test fixture"),
    );

    let error = upload(
        State(state),
        test_user(Uuid::now_v7()),
        Uuid::now_v7(),
        headers,
        Body::empty(),
    )
    .await
    .expect_err("declared over-limit bodies are rejected before reading");

    assert_eq!(
        error.into_response().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn streaming_file_size_limit_cleans_up_the_partial_upload() {
    let scratch = crate::photos::decode::testutil::ScratchDir::new("http-upload-limit");
    let mut state = test_state_with_storage(scratch.path().to_path_buf());
    Arc::get_mut(&mut state.config)
        .expect("state owns its config")
        .photos
        .max_upload_bytes = 1;
    let temp_dir = scratch.path().join("tmp");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .expect("create upload temp directory");
    let body = multipart_body("pv", &[("files", Some("large.jpg"), b"ab")]);
    let mut multipart = reader("pv", body, 1 << 20).await;
    let mut field = multipart
        .next_field()
        .await
        .expect("valid test fixture")
        .expect("file part");

    let error = ingest(
        &state,
        Uuid::now_v7(),
        Uuid::now_v7(),
        &mut field,
        &temp_dir,
        None,
    )
    .await
    .expect_err("file bytes over the per-file limit are refused");

    assert_eq!(
        error.into_response().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        std::fs::read_dir(&temp_dir)
            .expect("valid test fixture")
            .count(),
        0,
        "the rejected partial file is removed"
    );
}

#[tokio::test]
async fn an_upload_rate_of_zero_refuses_the_request() {
    let mut state = test_state();
    Arc::get_mut(&mut state.config)
        .expect("state owns its config")
        .photos
        .upload_rate_per_minute = 0;

    let error = upload(
        State(state),
        test_user(Uuid::now_v7()),
        Uuid::now_v7(),
        HeaderMap::new(),
        Body::empty(),
    )
    .await
    .expect_err("zero disables uploads");

    assert_eq!(
        error.into_response().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn upload_rejects_a_non_multipart_body() {
    let (state, _scratch) = upload_test_state();

    let error = upload(
        State(state),
        test_user(Uuid::now_v7()),
        Uuid::now_v7(),
        HeaderMap::new(),
        Body::empty(),
    )
    .await
    .expect_err("uploads require multipart/form-data");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_rejects_a_multipart_form_without_any_files() {
    let (state, scratch) = upload_test_state();
    let body = multipart_body("pv", &[("caption", None, b"hello")]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("valid test fixture"),
    );

    let error = upload(
        State(state),
        test_user(Uuid::now_v7()),
        Uuid::now_v7(),
        headers,
        Body::from(body),
    )
    .await
    .expect_err("ordinary fields are skipped but an upload still needs a file");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
    assert!(scratch.path().join("tmp").is_dir());
}

#[tokio::test]
async fn upload_rejects_unrecognized_file_bytes_and_removes_the_temporary_file() {
    let (state, scratch) = upload_test_state();
    let body = multipart_body(
        "pv",
        &[(
            "files",
            Some("looks-like-a-photo.jpg"),
            b"<html>not a photo</html>",
        )],
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("valid test fixture"),
    );

    let error = upload(
        State(state),
        test_user(Uuid::now_v7()),
        Uuid::now_v7(),
        headers,
        Body::from(body),
    )
    .await
    .expect_err("the file extension cannot make HTML into an image");

    assert_eq!(
        error.into_response().status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        std::fs::read_dir(scratch.path().join("tmp"))
            .expect("upload temp directory exists")
            .count(),
        0,
        "rejected upload leaves no temporary file"
    );
}

#[sqlx::test]
async fn upload_persists_a_valid_photo_and_queues_variant_generation(pool: PgPool) {
    let user = crate::db::repo::users::create(&pool, Some("Ada"), None)
        .await
        .expect("create owner");
    let voyage = crate::db::repo::voyages::create(
        &pool,
        &crate::db::repo::voyages::NewVoyage {
            owner_id: user.id,
            title: "Upload destination".into(),
            description: None,
            granularity: "exact".into(),
        },
    )
    .await
    .expect("create destination draft");
    let scratch = crate::photos::decode::testutil::ScratchDir::new("http-upload-valid");
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::from_sender(sender),
    );
    let original = crate::photos::decode::testutil::write_jpeg(&scratch, "valid-upload", 32, 24);
    let bytes = std::fs::read(original).expect("read JPEG fixture");
    let body = multipart_body("pv", &[("files", Some("ignored-name.jpg"), &bytes)]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("valid test fixture"),
    );

    let (status, Json(response)) = upload_to_voyage(
        State(state),
        test_user(user.id),
        Path(voyage.id),
        headers,
        Body::from(body),
    )
    .await
    .expect("valid image upload succeeds");

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(response.photos.len(), 1);
    let photo = &response.photos[0];
    assert_eq!(photo.original_format, "jpeg");
    assert_eq!((photo.width, photo.height), (32, 24));
    assert_eq!(photo.variant_status, STATUS_PENDING);

    let job = receiver.recv().await.expect("upload enqueued variant work");
    assert_eq!(job.photo_id, photo.id);
    let persisted_status =
        sqlx::query_scalar!("SELECT variant_status FROM photos WHERE id = $1", photo.id,)
            .fetch_one(&pool)
            .await
            .expect("photo row persisted");
    assert_eq!(persisted_status, STATUS_PENDING);
    let members = crate::db::repo::voyages::photos_for_voyage(&pool, voyage.id)
        .await
        .expect("read voyage");
    assert_eq!(
        members.len(),
        1,
        "an accepted upload already belongs to its voyage"
    );
    assert_eq!(members[0].photo_id, photo.id);
}

#[tokio::test]
async fn unscoped_photo_library_routes_are_not_available() {
    for method in ["GET", "POST"] {
        assert_eq!(
            status_of(method, "/api/photos").await,
            StatusCode::NOT_FOUND
        );
    }
}

#[sqlx::test]
async fn a_closed_worker_refuses_upload_before_persisting_a_photo(pool: PgPool) {
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let scratch = crate::photos::decode::testutil::ScratchDir::new("http-upload-closed-worker");
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );
    let original = crate::photos::decode::testutil::write_jpeg(&scratch, "closed-worker", 16, 12);
    let bytes = std::fs::read(original).expect("read JPEG fixture");
    let body = multipart_body("pv", &[("files", Some("photo.jpg"), &bytes)]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("valid test fixture"),
    );

    let error = upload(
        State(state.clone()),
        test_user(user.id),
        photo_voyage::fixture_voyage(&pool, user.id).await,
        headers,
        Body::from(body),
    )
    .await
    .expect_err("a closed worker cannot accept the new job");
    assert_eq!(
        error.into_response().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );

    let photos = crate::db::repo::photos::list_for_owner(&pool, user.id, None, 10)
        .await
        .expect("the committed photo remains in its voyage");
    assert!(photos.is_empty());
}

#[sqlx::test]
async fn voyage_photo_list_sets_and_omits_the_next_cursor(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let older = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 1)).await;
    let newer = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 2)).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("photo-list");
    let state = test_state_with(
        pool,
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );
    let Json(full_page) = list(
        State(state.clone()),
        test_user(owner.id),
        Path(owner.id),
        Query(ListQuery {
            before: None,
            limit: Some(1),
        }),
    )
    .await
    .expect("listing succeeds");
    assert_eq!(full_page.photos.len(), 1);
    assert_eq!(full_page.photos[0].id, newer.id);
    assert_eq!(full_page.next_before, Some(newer.id));

    let Json(short_page) = list(
        State(state),
        test_user(owner.id),
        Path(owner.id),
        Query(ListQuery {
            before: Some(newer.id),
            limit: Some(2),
        }),
    )
    .await
    .expect("older page succeeds");
    assert_eq!(short_page.photos.len(), 1);
    assert_eq!(short_page.photos[0].id, older.id);
    assert_eq!(short_page.next_before, None);
}

#[sqlx::test]
async fn photo_metadata_is_owner_only_and_missing_ids_are_not_found(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let stranger = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create stranger");
    let photo = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 3)).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("photo-metadata");
    let state = test_state_with(
        pool,
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );

    let Json(response) = metadata(State(state.clone()), test_user(owner.id), Path(photo.id))
        .await
        .expect("owner can read metadata");
    assert_eq!(response.id, photo.id);
    assert_eq!(response.width, 32);

    assert!(matches!(
        metadata(State(state.clone()), test_user(stranger.id), Path(photo.id)).await,
        Err(Rejection::App(AppError::NotFound))
    ));
    assert!(matches!(
        metadata(State(state), test_user(owner.id), Path(Uuid::now_v7())).await,
        Err(Rejection::App(AppError::NotFound))
    ));
}

#[sqlx::test]
async fn deleting_photos_removes_only_the_last_shared_content_file(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let unique = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 4)).await;
    let shared_a = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 5)).await;
    let shared_b = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 5)).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("photo-delete");
    let state = test_state_with(
        pool,
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );
    let unremovable_original = crate::photos::storage::original_path(
        &state.config.photos.storage_root,
        &crate::photos::storage::ContentHash::try_from(unique.content_hash.clone())
            .expect("valid fixture hash"),
    );
    std::fs::create_dir_all(unremovable_original.parent().expect("valid test fixture"))
        .expect("create original shard");
    std::fs::create_dir(&unremovable_original).expect("directory blocks original-file removal");
    std::fs::write(unremovable_original.join("keep"), b"still referenced")
        .expect("make the directory non-empty");

    assert_eq!(
        destroy(State(state.clone()), test_user(owner.id), Path(unique.id))
            .await
            .expect("delete unique photo"),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        destroy(State(state.clone()), test_user(owner.id), Path(shared_a.id))
            .await
            .expect("delete one shared photo"),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        destroy(State(state.clone()), test_user(owner.id), Path(shared_b.id))
            .await
            .expect("delete final shared photo"),
        StatusCode::NO_CONTENT
    );
    assert!(matches!(
        destroy(State(state.clone()), test_user(owner.id), Path(unique.id)).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        destroy(State(state), test_user(Uuid::now_v7()), Path(shared_b.id)).await,
        Err(AppError::NotFound)
    ));
}

#[sqlx::test]
async fn image_serving_sets_headers_and_honors_a_matching_etag(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let photo = create_photo_for_test(&pool, owner.id, &format!("{:064x}", 6)).await;
    crate::db::repo::photos::set_variant_status(&pool, photo.id, STATUS_READY)
        .await
        .expect("mark variants ready");
    let scratch = crate::photos::decode::testutil::ScratchDir::new("photo-image");
    let state = test_state_with(
        pool,
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );
    let path = crate::photos::storage::variant_path(
        &state.config.photos.storage_root,
        &crate::photos::storage::ContentHash::try_from(photo.content_hash.clone())
            .expect("valid fixture hash"),
        VariantKind::Thumb,
    );
    std::fs::create_dir_all(path.parent().expect("valid test fixture"))
        .expect("create variant shard");
    std::fs::write(&path, b"webp variant bytes").expect("write variant fixture");

    assert!(matches!(
        image(
            State(state.clone()),
            MaybeUser(Some(test_user(owner.id))),
            Path((photo.id, "unknown-variant".to_owned())),
            HeaderMap::new(),
        )
        .await,
        Err(Rejection::App(AppError::NotFound))
    ));

    let response = image(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Path((photo.id, "thumb".to_owned())),
        HeaderMap::new(),
    )
    .await
    .expect("owner can read their ready variant");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .expect("small variant"),
        &b"webp variant bytes"[..]
    );

    let tag = etag(&photo.content_hash, VariantKind::Thumb);
    let mut conditional = HeaderMap::new();
    conditional.insert(
        header::IF_NONE_MATCH,
        tag.parse().expect("valid test fixture"),
    );
    let response = image(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Path((photo.id, "thumb".to_owned())),
        conditional,
    )
    .await
    .expect("matching etag is a successful conditional response");
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);

    assert!(matches!(
        image(
            State(state.clone()),
            MaybeUser(None),
            Path((photo.id, "thumb".to_owned())),
            HeaderMap::new(),
        )
        .await,
        Err(Rejection::App(AppError::NotFound))
    ));

    std::fs::remove_file(&path).expect("remove ready variant fixture");
    assert!(matches!(
        image(
            State(state.clone()),
            MaybeUser(Some(test_user(owner.id))),
            Path((photo.id, "thumb".to_owned())),
            HeaderMap::new(),
        )
        .await,
        Err(Rejection::App(AppError::NotFound))
    ));

    std::fs::create_dir(&path).expect("replace the variant file with a directory");
    let error = image(
        State(state),
        MaybeUser(Some(test_user(owner.id))),
        Path((photo.id, "thumb".to_owned())),
        HeaderMap::new(),
    )
    .await
    .expect_err("a directory cannot be served as an image");
    assert_eq!(
        error.into_response().status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

fn header_of(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Checks image response headers without a database or filesystem.
#[test]
fn a_served_image_carries_the_security_headers() {
    let tag = etag(&"a".repeat(64), VariantKind::Display);
    let headers = image_headers(&tag, 4096, None);

    assert_eq!(
        header_of(&headers, header::CONTENT_TYPE),
        Some("image/webp"),
        "the type is ours, never the upload's"
    );
    assert_eq!(
        header_of(&headers, header::X_CONTENT_TYPE_OPTIONS),
        Some("nosniff")
    );
    assert_eq!(
        header_of(&headers, header::CACHE_CONTROL),
        Some("private, no-cache, must-revalidate")
    );
    assert_eq!(header_of(&headers, header::ETAG), Some(tag.as_str()));
    assert_eq!(header_of(&headers, header::CONTENT_LENGTH), Some("4096"));

    let disposition =
        header_of(&headers, header::CONTENT_DISPOSITION).expect("content disposition");
    assert!(disposition.starts_with("inline; filename="));
    assert!(
        disposition.ends_with(".webp\""),
        "the filename is ours: {disposition}"
    );
}

#[test]
fn last_modified_is_an_http_date() {
    let headers = image_headers("\"tag\"", 1, Some(SystemTime::UNIX_EPOCH));
    assert_eq!(
        header_of(&headers, header::LAST_MODIFIED),
        Some("Thu, 01 Jan 1970 00:00:00 GMT")
    );
}

/// ETags differ by original content hash and variant name.
#[test]
fn etags_are_content_derived() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);

    assert_eq!(etag(&a, VariantKind::Thumb), etag(&a, VariantKind::Thumb));
    assert_ne!(etag(&a, VariantKind::Thumb), etag(&a, VariantKind::Full));
    assert_ne!(etag(&a, VariantKind::Thumb), etag(&b, VariantKind::Thumb));
}

/// A variant that does not exist yet is an ordinary race — an upload
/// returns before its variants do — and must never be a 500.
#[test]
fn a_pending_variant_is_not_a_server_error() {
    let status = availability(STATUS_PENDING)
        .expect_err("pending is not servable")
        .into_response()
        .status();
    assert_eq!(status, StatusCode::CONFLICT);

    for terminal in [STATUS_FAILED, "something-a-later-migration-adds"] {
        let status = availability(terminal)
            .expect_err("not servable")
            .into_response()
            .status();
        assert_eq!(status, StatusCode::NOT_FOUND, "{terminal}");
    }

    assert!(availability(STATUS_READY).is_ok());
}

/// The design's rejection branches, which `AppError` alone cannot express.
#[test]
fn domain_failures_keep_their_status() {
    let cases = [
        (PhotoError::TooLarge, StatusCode::PAYLOAD_TOO_LARGE),
        (PhotoError::NotAnImage, StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (PhotoError::TooManyPixels, StatusCode::UNPROCESSABLE_ENTITY),
        (
            PhotoError::Undecodable("truncated".to_owned()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            PhotoError::Unencodable("libwebp".to_owned()),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ];

    for (err, expected) in cases {
        assert_eq!(Rejection::from(err).into_response().status(), expected);
    }
}

/// 429 with a `Retry-After`, not a reset connection (T12).
#[test]
fn the_rate_limiter_refuses_with_a_retry_after() {
    let limiter = crate::photos::rate_limit::UploadLimiter::default();
    let now = Instant::now();
    let take_upload_token = |user, rate| {
        limiter
            .take_at(user, rate, now)
            .map_err(Rejection::rate_limited)
    };
    let user = Uuid::now_v7();

    // Two per minute: two succeed, the third does not.
    assert!(take_upload_token(user, 2).is_ok());
    assert!(take_upload_token(user, 2).is_ok());

    let response = take_upload_token(user, 2)
        .expect_err("the third upload is over budget")
        .into_response();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("retry-after is set");
    assert!(
        (1..=60).contains(&retry),
        "implausible retry-after: {retry}"
    );
}

/// Buckets are per user: one account exhausting its budget must not refuse
/// anybody else's upload.
#[test]
fn the_rate_limiter_is_per_user() {
    let limiter = crate::photos::rate_limit::UploadLimiter::default();
    let now = Instant::now();
    let take_upload_token = |user, rate| {
        limiter
            .take_at(user, rate, now)
            .map_err(Rejection::rate_limited)
    };
    let noisy = Uuid::now_v7();
    let quiet = Uuid::now_v7();

    assert!(take_upload_token(noisy, 1).is_ok());
    assert!(take_upload_token(noisy, 1).is_err());
    assert!(take_upload_token(quiet, 1).is_ok());
}

// -- the multipart reader ------------------------------------------------

fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, filename, content) in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"").as_bytes(),
        );
        if let Some(filename) = filename {
            body.extend_from_slice(format!("; filename=\"{filename}\"").as_bytes());
        }
        body.extend_from_slice(b"\r\nContent-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

async fn reader(boundary: &str, body: Vec<u8>, limit: u64) -> Multipart {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={boundary}")
            .parse()
            .expect("content type"),
    );
    read_multipart(headers, Body::from(body), limit)
        .await
        .expect("valid boundary")
}

async fn collect(multipart: &mut Multipart) -> Result<Vec<(Option<String>, Vec<u8>)>, Rejection> {
    let mut parts = Vec::new();
    while let Some(mut part) = multipart.next_field().await.map_err(multipart_error)? {
        check_part_headers(&part)?;
        let filename = part.file_name().map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = part.chunk().await.map_err(multipart_error)? {
            bytes.extend_from_slice(&chunk);
        }
        parts.push((filename, bytes));
    }
    Ok(parts)
}

#[tokio::test]
async fn axum_multipart_preserves_files_across_every_chunk_boundary() {
    use axum::body::Bytes;
    let content = b"before--pv still data after";
    let body = multipart_body(
        "pv",
        &[
            ("caption", None, b"caption"),
            ("files", Some("one.jpg"), content),
            ("files", Some("two.png"), b"second"),
        ],
    );
    for size in [1, 2, 3, 7, 13, 64, 1000] {
        let chunks: Vec<_> = body.chunks(size).map(Bytes::copy_from_slice).collect();
        let stream = Body::from_stream(tokio_stream::iter(
            chunks.into_iter().map(Ok::<_, std::convert::Infallible>),
        ));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=pv".parse().expect("header"),
        );
        let mut multipart = read_multipart(headers, stream, 1 << 20)
            .await
            .expect("parser");
        let parts = collect(&mut multipart).await.expect("well-formed body");
        assert_eq!(
            parts,
            vec![
                (None, b"caption".to_vec()),
                (Some("one.jpg".into()), content.to_vec()),
                (Some("two.png".into()), b"second".to_vec())
            ]
        );
    }
}

#[tokio::test]
async fn axum_multipart_rejects_malformed_truncated_and_oversized_bodies() {
    let valid = multipart_body("pv", &[("files", Some("one.jpg"), b"payload")]);
    for body in [
        b"not multipart".to_vec(),
        b"--pv\r\nnot-a-header\r\n\r\n".to_vec(),
        b"--pv\r\n\xff: value\r\n\r\n".to_vec(),
        valid[..valid.len() - 10].to_vec(),
    ] {
        let error = collect(&mut reader("pv", body, 1 << 20).await)
            .await
            .expect_err("malformed request");
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
    }
    let error = collect(&mut reader("pv", valid, 16).await)
        .await
        .expect_err("framing counts against limit");
    assert_eq!(
        error.into_response().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let large_header = multipart_body("pv", &[("files", Some(&"x".repeat(4096)), b"data")]);
    assert_eq!(
        collect(&mut reader("pv", large_header, 1 << 20).await)
            .await
            .expect_err("headers bounded")
            .into_response()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn content_length_ignores_invalid_values() {
    let mut headers = HeaderMap::new();
    assert_eq!(content_length(&headers), None);

    headers.insert(
        header::CONTENT_LENGTH,
        "123".parse().expect("valid test fixture"),
    );
    assert_eq!(content_length(&headers), Some(123));

    headers.insert(
        header::CONTENT_LENGTH,
        "not-a-number".parse().expect("valid test fixture"),
    );
    assert_eq!(content_length(&headers), None);

    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_bytes(b"\xff").expect("valid test fixture"),
    );
    assert_eq!(content_length(&headers), None);
}

#[test]
fn database_errors_become_database_rejections() {
    let rejection = Rejection::from(sqlx::Error::PoolTimedOut);
    assert_eq!(
        rejection.into_response().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[sqlx::test]
async fn upload_and_list_refuse_missing_and_foreign_voyages(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let stranger = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("stranger");
    let voyage_id = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("voyage-upload-ownership");
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );
    for (user, id) in [(stranger.id, voyage_id), (owner.id, Uuid::now_v7())] {
        let error = upload_to_voyage(
            State(state.clone()),
            test_user(user),
            Path(id),
            HeaderMap::new(),
            Body::empty(),
        )
        .await
        .expect_err("no destination access");
        assert_eq!(error.into_response().status(), StatusCode::NOT_FOUND);
        let error = list(
            State(state.clone()),
            test_user(user),
            Path(id),
            Query(ListQuery {
                before: None,
                limit: None,
            }),
        )
        .await
        .expect_err("no metadata access");
        assert_eq!(error.into_response().status(), StatusCode::NOT_FOUND);
    }
    assert!(
        !scratch.path().join("tmp").exists(),
        "authorization happens before reading bytes"
    );
}

#[sqlx::test]
async fn a_voyage_deleted_before_ingest_cannot_leave_an_unattached_photo(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let scratch = crate::photos::decode::testutil::ScratchDir::new("deleted-voyage-upload");
    let original = crate::photos::decode::testutil::write_jpeg(&scratch, "upload", 16, 12);
    let bytes = std::fs::read(original).expect("JPEG");
    let body = multipart_body("pv", &[("files", Some("upload.jpg"), &bytes)]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv".parse().expect("header"),
    );
    let (sender, _receiver) = tokio::sync::mpsc::channel(1);
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::from_sender(sender),
    );
    let error = upload(
        State(state),
        test_user(owner.id),
        Uuid::now_v7(),
        headers,
        Body::from(body),
    )
    .await
    .expect_err("destination disappeared");
    assert_eq!(error.into_response().status(), StatusCode::NOT_FOUND);
    assert_eq!(
        std::fs::read_dir(scratch.path().join("tmp"))
            .expect("temp directory")
            .count(),
        0
    );
    assert!(
        crate::db::repo::photos::list_for_owner(&pool, owner.id, None, 10)
            .await
            .expect("photos")
            .is_empty()
    );
}

/// Uses a committed HEVC/HEIC file: a missing decoder must fail, never skip.
#[sqlx::test]
async fn heic_upload_generates_and_serves_every_variant(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, Some("HEIC owner"), None)
        .await
        .expect("owner");
    let voyage_id = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("heic-upload-regression");
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::from_sender(sender),
    );
    let bytes = include_bytes!("../../tests/fixtures/upload-regression.heic");
    let body = multipart_body("pv", &[("files", Some("photo.heic"), bytes)]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("multipart header"),
    );
    let (status, Json(result)) = upload_to_voyage(
        State(state.clone()),
        test_user(owner.id),
        Path(voyage_id),
        headers,
        Body::from(body),
    )
    .await
    .expect("HEIC upload succeeds");
    assert_eq!(status, StatusCode::CREATED);
    let photo = &result.photos[0];
    assert_eq!(photo.original_format, "heic");
    assert_eq!((photo.width, photo.height), (64, 48));
    assert_eq!(photo.variant_status, STATUS_PENDING);
    let pending = image(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Path((photo.id, "thumb".into())),
        HeaderMap::new(),
    )
    .await
    .expect_err("pending variants are not served")
    .into_response();
    assert_eq!(pending.status(), StatusCode::CONFLICT);
    let job = receiver.recv().await.expect("queued job");
    crate::photos::worker::process(&pool, &state.config, &job)
        .await
        .expect("HEIC variants generated");
    let updated = photos::find_by_id(&pool, photo.id)
        .await
        .expect("read photo")
        .expect("photo exists");
    assert_eq!(updated.variant_status, STATUS_READY);
    for kind in VariantKind::ALL {
        let response = image(
            State(state.clone()),
            MaybeUser(Some(test_user(owner.id))),
            Path((photo.id, kind.as_str().into())),
            HeaderMap::new(),
        )
        .await
        .expect("serve generated variant");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
        let served = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("variant bytes");
        let decoded = ::image::load_from_memory_with_format(&served, ::image::ImageFormat::WebP)
            .expect("served variant is readable WebP");
        assert_eq!((decoded.width(), decoded.height()), (64, 48));
    }
    assert_eq!(
        crate::db::repo::voyages::photos_for_voyage(&pool, voyage_id)
            .await
            .expect("voyage photos")
            .len(),
        1
    );
}

#[sqlx::test]
async fn corrupt_heic_upload_is_rejected_without_a_photo_or_job(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage_id = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("corrupt-heic-regression");
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::from_sender(sender),
    );
    // Keep a real HEIC signature but truncate before the image payload.
    let fixture = include_bytes!("../../tests/fixtures/upload-regression.heic");
    let body = multipart_body("pv", &[("files", Some("corrupt.heic"), &fixture[..32])]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("multipart header"),
    );
    let error = upload_to_voyage(
        State(state),
        test_user(owner.id),
        Path(voyage_id),
        headers,
        Body::from(body),
    )
    .await
    .expect_err("corrupt HEIC rejected")
    .into_response();
    assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(error.into_body(), usize::MAX)
        .await
        .expect("error body");
    let error: serde_json::Value = serde_json::from_slice(&body).expect("JSON error");
    assert_eq!(error["code"], "unprocessable_image");
    assert!(
        photos::list_for_voyage(&pool, owner.id, voyage_id, None, 10)
            .await
            .expect("photo list")
            .is_empty()
    );
    assert!(
        receiver.try_recv().is_err(),
        "rejected image never queues work"
    );
}

#[sqlx::test]
async fn heic_conversion_failure_marks_the_photo_failed(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage_id = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("heic-worker-failure");
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::from_sender(sender),
    );
    let fixture = include_bytes!("../../tests/fixtures/upload-regression.heic");
    let body = multipart_body("pv", &[("files", Some("photo.heic"), fixture)]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("multipart header"),
    );
    let (_, Json(result)) = upload_to_voyage(
        State(state.clone()),
        test_user(owner.id),
        Path(voyage_id),
        headers,
        Body::from(body),
    )
    .await
    .expect("valid upload");
    let photo = photos::find_by_id(&pool, result.photos[0].id)
        .await
        .expect("photo lookup")
        .expect("photo");
    let original = crate::photos::storage::original_path(
        scratch.path(),
        &ContentHash::try_from(photo.content_hash).expect("valid fixture hash"),
    );
    std::fs::write(original, &fixture[..32])
        .expect("simulate original corruption before conversion");
    let job = receiver.recv().await.expect("queued job");
    assert!(
        crate::photos::worker::process(&pool, &state.config, &job)
            .await
            .is_err()
    );
    assert_eq!(
        photos::find_by_id(&pool, photo.id)
            .await
            .expect("photo lookup")
            .expect("photo remains in voyage")
            .variant_status,
        STATUS_FAILED
    );
    let response = image(
        State(state),
        MaybeUser(Some(test_user(owner.id))),
        Path((photo.id, "thumb".into())),
        HeaderMap::new(),
    )
    .await
    .expect_err("failed conversion never serves an original")
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn failed_delete_commit_keeps_referenced_files(pool: PgPool) {
    let scratch = crate::photos::decode::testutil::ScratchDir::new("delete-commit");
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let photo = create_photo_for_test(&pool, owner.id, &"a".repeat(64)).await;
    let path = storage::original_path(
        scratch.path(),
        &ContentHash::try_from(photo.content_hash.clone()).expect("valid fixture hash"),
    );
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(&path, b"original").expect("original");
    sqlx::raw_sql(sqlx::AssertSqlSafe("CREATE FUNCTION refuse_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'commit refused'; END $$; CREATE CONSTRAINT TRIGGER refuse_delete AFTER DELETE ON photos DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse_delete();"))
        .execute(&pool).await.expect("deferred failure");
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
    );
    assert!(
        destroy(State(state), test_user(owner.id), Path(photo.id))
            .await
            .is_err()
    );
    assert!(
        photos::find_by_id(&pool, photo.id)
            .await
            .expect("read")
            .is_some()
    );
    assert!(
        path.exists(),
        "failed commit must preserve referenced original"
    );
}

#[sqlx::test]
async fn retrying_an_upload_key_returns_the_same_photo(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("idempotent-upload");
    let original = crate::photos::decode::testutil::write_jpeg(&scratch, "photo", 16, 12);
    let bytes = std::fs::read(original).expect("JPEG bytes");
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::from_sender(sender),
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv"
            .parse()
            .expect("content type"),
    );
    headers.insert(
        "idempotency-key",
        Uuid::new_v4().to_string().parse().expect("upload key"),
    );
    let mut ids = Vec::new();
    for _ in 0..2 {
        let body = multipart_body("pv", &[("files", Some("photo.jpg"), &bytes)]);
        let (_, Json(response)) = upload(
            State(state.clone()),
            test_user(owner.id),
            voyage,
            headers.clone(),
            Body::from(body),
        )
        .await
        .expect("accepted upload");
        ids.push(response.photos[0].id);
    }
    let changed = crate::photos::decode::testutil::write_jpeg(&scratch, "different", 20, 16);
    let changed = std::fs::read(changed).expect("different JPEG");
    let error = upload(
        State(state.clone()),
        test_user(owner.id),
        voyage,
        headers.clone(),
        Body::from(multipart_body(
            "pv",
            &[("files", Some("different.jpg"), &changed)],
        )),
    )
    .await
    .expect_err("a retry key cannot represent different content");
    assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
    for invalid in [
        HeaderValue::from_static("invalid"),
        HeaderValue::from_bytes(&[0xff]).expect("opaque header"),
    ] {
        headers.insert("idempotency-key", invalid);
        let error = upload(
            State(state.clone()),
            test_user(owner.id),
            voyage,
            headers.clone(),
            Body::from(multipart_body(
                "pv",
                &[("files", Some("photo.jpg"), &bytes)],
            )),
        )
        .await
        .expect_err("key must be a UUID");
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(ids[0], ids[1]);
    assert!(receiver.try_recv().is_ok());
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn a_variant_path_must_be_a_regular_file() {
    let scratch = crate::photos::decode::testutil::ScratchDir::new("variant-directory");
    let error = variant_body(scratch.path())
        .await
        .expect_err("directory is not an image");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[sqlx::test]
async fn photo_deletion_queues_files_without_removing_them_in_the_request(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let photo = create_photo_for_test(&pool, owner.id, &"d".repeat(64)).await;
    let scratch = crate::photos::decode::testutil::ScratchDir::new("background-photo-delete");
    let path = storage::original_path(
        scratch.path(),
        &ContentHash::try_from(photo.content_hash.clone()).expect("hash"),
    );
    std::fs::create_dir_all(path.parent().expect("parent")).expect("shard");
    std::fs::write(&path, b"original").expect("original");
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_owned(),
        crate::photos::worker::VariantQueue::detached(),
    );
    destroy(State(state), test_user(owner.id), Path(photo.id))
        .await
        .expect("delete");
    assert!(path.exists(), "HTTP success precedes file removal");
    assert!(
        photos::find_by_id(&pool, photo.id)
            .await
            .expect("lookup")
            .is_none()
    );
    crate::photos::cleanup::sweep(&pool, scratch.path())
        .await
        .expect("background removal");
    assert!(!path.exists());
}

#[tokio::test]
async fn canceling_an_upload_removes_its_partially_written_file() {
    let (state, scratch) = upload_test_state();
    let jpeg = crate::photos::decode::testutil::write_jpeg(&scratch, "source", 32, 24);
    let mut bytes = b"--pv\r\nContent-Disposition: form-data; name=\"files\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n".to_vec();
    bytes.extend(std::fs::read(jpeg).expect("jpeg"));
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    sender.send(Ok(bytes)).await.expect("partial body");
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv".parse().expect("header"),
    );
    let request = tokio::spawn(upload(
        State(state),
        test_user(Uuid::now_v7()),
        Uuid::now_v7(),
        headers,
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver)),
    ));
    let temp = scratch.path().join("tmp");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if std::fs::read_dir(&temp).ok().is_some_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.metadata().is_ok_and(|m| m.len() > 0))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("partial upload reaches disk");
    request.abort();
    assert!(request.await.expect_err("canceled").is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while std::fs::read_dir(&temp)
            .expect("temporary directory")
            .count()
            != 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancellation removes the temporary upload");
    drop(sender);
}
