use crate::photo_voyage;
use std::sync::Arc;

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

#[tokio::test]
async fn multipart_frame_reader_skips_trailers_and_keeps_reading_data() {
    use axum::body::{Body, Bytes};
    use http_body::Frame;
    use http_body_util::StreamBody;
    use std::convert::Infallible;

    let frames = tokio_stream::iter([
        Ok::<_, Infallible>(Frame::trailers(HeaderMap::new())),
        Ok(Frame::data(Bytes::from_static(b"after trailers"))),
    ]);
    let mut body = Body::new(StreamBody::new(frames));

    assert_eq!(
        next_frame(&mut body).await.unwrap().unwrap(),
        Bytes::from_static(b"after trailers")
    );
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

    // Note what is *not* in this list: `GET /api/photos/{id}/{variant}`.
    //
    // Serving the bytes is deliberately anonymous-capable, because every
    // voyage is public (D6) and a published voyage whose images 404 for
    // anyone but its author is a public gallery with the pictures painted
    // out. That route authorizes per photograph — owner, or attached to a
    // published voyage — rather than per session; see `viewable` and
    // `tests/photos_public_access.rs`.
    //
    // The *metadata* routes stay session-gated. They return exact
    // coordinates to an owner, and the granularity boundary lives in
    // `http::voyages`; opening them would route around it.
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
        declared.to_string().parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv".parse().unwrap(),
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
    let mut multipart = reader("pv", body, 1 << 20);
    multipart.next_part().await.unwrap().expect("file part");

    let error = ingest(
        &state,
        Uuid::now_v7(),
        Uuid::now_v7(),
        &mut multipart,
        &temp_dir,
    )
    .await
    .expect_err("file bytes over the per-file limit are refused");

    assert_eq!(
        error.into_response().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        std::fs::read_dir(&temp_dir).unwrap().count(),
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
        "multipart/form-data; boundary=pv".parse().unwrap(),
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
        "multipart/form-data; boundary=pv".parse().unwrap(),
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
        crate::photos::worker::VariantQueue { sender },
    );
    let original = crate::photos::decode::testutil::write_jpeg(&scratch, "valid-upload", 32, 24);
    let bytes = std::fs::read(original).expect("read JPEG fixture");
    let body = multipart_body("pv", &[("files", Some("ignored-name.jpg"), &bytes)]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=pv".parse().unwrap(),
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
async fn upload_reports_a_closed_worker_after_persisting_the_pending_photo(pool: PgPool) {
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
        "multipart/form-data; boundary=pv".parse().unwrap(),
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
    assert_eq!(photos.len(), 1);
    assert_eq!(photos[0].variant_status, STATUS_PENDING);
    assert!(
        crate::photos::storage::original_path(
            &state.config.photos.storage_root,
            &crate::photos::storage::ContentHash(photos[0].content_hash.clone()),
        )
        .is_file()
    );
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
        &crate::photos::storage::ContentHash(unique.content_hash.clone()),
    );
    std::fs::create_dir_all(unremovable_original.parent().unwrap()).expect("create original shard");
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
        &crate::photos::storage::ContentHash(photo.content_hash.clone()),
        VariantKind::Thumb,
    );
    std::fs::create_dir_all(path.parent().unwrap()).expect("create variant shard");
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
    conditional.insert(header::IF_NONE_MATCH, tag.parse().unwrap());
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

/// T9's header set, asserted without a database and without a file — which
/// is what makes it a test that will still be run in a year.
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
        Some(IMAGE_CACHE_CONTROL)
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

/// The tag is derived from content and variant, so two variants of one
/// photo never collide and one variant of two photos never does either.
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

fn reader(boundary_value: &str, body: Vec<u8>, limit: u64) -> Multipart {
    Multipart::new(
        &format!("multipart/form-data; boundary={boundary_value}"),
        Body::from(body),
        limit,
    )
    .expect("a valid boundary")
}

/// The same body delivered `frame_size` bytes at a time, which is the shape
/// a real network body has and an in-memory one never does.
fn chunked_reader(boundary_value: &str, body: Vec<u8>, frame_size: usize) -> Multipart {
    let frames: Vec<_> = body
        .chunks(frame_size)
        .map(Bytes::copy_from_slice)
        .collect();

    Multipart::new(
        &format!("multipart/form-data; boundary={boundary_value}"),
        Body::from_stream(tokio_stream::iter(
            frames.into_iter().map(Ok::<_, std::convert::Infallible>),
        )),
        u64::MAX,
    )
    .expect("a valid boundary")
}

async fn collect(multipart: &mut Multipart) -> Vec<(Option<String>, Vec<u8>)> {
    let mut parts = Vec::new();
    while let Some(part) = multipart.next_part().await.expect("a well-formed body") {
        let mut content = Vec::new();
        while let Some(chunk) = multipart.next_chunk().await.expect("a well-formed part") {
            content.extend_from_slice(&chunk);
        }
        parts.push((part.filename, content));
    }
    parts
}

#[tokio::test]
async fn several_files_in_one_request_are_read_in_order() {
    let raw = multipart_body(
        "pv",
        &[
            ("caption", None, b"not a file"),
            ("files", Some("one.jpg"), b"\xff\xd8\xff\xe0one"),
            ("files", Some("two.png"), b"\x89PNGtwo"),
        ],
    );

    let parts = collect(&mut reader("pv", raw, 1 << 20)).await;

    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].0, None, "an ordinary field has no filename");
    assert_eq!(parts[1].0.as_deref(), Some("one.jpg"));
    assert_eq!(parts[1].1, b"\xff\xd8\xff\xe0one");
    assert_eq!(parts[2].1, b"\x89PNGtwo");
}

/// The case a naive split on the boundary gets wrong: image bytes that
/// happen to contain the boundary text without the CRLF that precedes a
/// real delimiter.
#[tokio::test]
async fn a_boundary_inside_the_data_is_not_a_delimiter() {
    let content = b"before--pv still the same file after".to_vec();
    let raw = multipart_body("pv", &[("files", Some("a.bin"), &content)]);

    let parts = collect(&mut reader("pv", raw, 1 << 20)).await;

    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].1, content);
}

/// Every delimiter, header line and part boundary can land across a frame
/// boundary. Sweeping the frame size exercises all of those splits, and
/// one-byte frames exercise every one of them at once.
#[tokio::test]
async fn a_delimiter_split_across_frames_still_matches() {
    let content: Vec<u8> = (0u8..=255).cycle().take(3000).collect();
    let raw = multipart_body(
        "pv-boundary",
        &[
            ("files", Some("a.bin"), &content),
            ("files", Some("b.bin"), b"second"),
        ],
    );

    for frame_size in [1, 2, 3, 7, 13, 64, 1000] {
        let parts = collect(&mut chunked_reader("pv-boundary", raw.clone(), frame_size)).await;

        assert_eq!(parts.len(), 2, "frame size {frame_size}");
        assert_eq!(parts[0].0.as_deref(), Some("a.bin"), "{frame_size}");
        assert_eq!(parts[0].1, content, "frame size {frame_size}");
        assert_eq!(parts[1].1, b"second", "frame size {frame_size}");
    }
}

/// T12's mid-stream half: a body that exceeds the cap is refused with a 413
/// even though nothing declared a `Content-Length`.
#[tokio::test]
async fn an_oversized_body_is_refused_with_413() {
    let raw = multipart_body("pv", &[("files", Some("a.bin"), &vec![0u8; 4096])]);
    let mut multipart = reader("pv", raw, 512);

    let status = loop {
        match multipart.next_part().await {
            Ok(Some(_)) => while let Ok(Some(_)) = multipart.next_chunk().await {},
            Ok(None) => panic!("the whole body was accepted"),
            Err(rejection) => break rejection.into_response().status(),
        }
    };

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

/// A truncated upload must not be stored as though it were whole.
#[tokio::test]
async fn a_body_with_no_closing_delimiter_is_refused() {
    let mut raw = multipart_body("pv", &[("files", Some("a.bin"), b"0123456789")]);
    raw.truncate(raw.len() - 12);

    let mut multipart = reader("pv", raw, 1 << 20);
    multipart.next_part().await.expect("headers are intact");

    let result = loop {
        match multipart.next_chunk().await {
            Ok(Some(_)) => continue,
            other => break other,
        }
    };

    assert!(result.is_err(), "a truncated body was accepted as complete");
}

#[tokio::test]
async fn a_body_without_any_delimiter_is_malformed() {
    let mut multipart = reader("pv", b"not multipart".to_vec(), 1 << 20);

    let error = multipart
        .next_part()
        .await
        .expect_err("a body without a delimiter is malformed");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_delimiter_line_with_unexpected_text_is_malformed() {
    let body = b"--pv\r\nunexpected\r\n".to_vec();
    let mut multipart = reader("pv", body, 1 << 20);

    let error = multipart
        .next_part()
        .await
        .expect_err("the delimiter line only permits closing marks or whitespace");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_header_line_without_a_colon_is_malformed() {
    let body = b"--pv\r\nnot-a-header\r\n\r\n".to_vec();
    let mut multipart = reader("pv", body, 1 << 20);

    let error = multipart
        .next_part()
        .await
        .expect_err("part headers must have a name and value");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_body_stream_error_becomes_a_bad_request() {
    let body = Body::from_stream(tokio_stream::iter([Err::<Bytes, _>(
        std::io::Error::other("synthetic body failure"),
    )]));
    let mut multipart =
        Multipart::new("multipart/form-data; boundary=pv", body, 1 << 20).expect("valid boundary");

    let error = multipart
        .next_part()
        .await
        .expect_err("failed body frames are rejected");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_utf8_headers_are_ignored_and_parsing_continues() {
    let body = b"--pv\r\n\xff: value\r\n\r\ncontent\r\n--pv--\r\n".to_vec();
    let mut multipart = reader("pv", body, 1 << 20);

    let part = multipart
        .next_part()
        .await
        .expect("valid multipart structure")
        .expect("part follows delimiter");
    assert_eq!(part.filename, None);
    assert_eq!(
        multipart.next_chunk().await.unwrap().as_deref(),
        Some(&b"content"[..])
    );
}

#[tokio::test]
async fn multipart_reader_remains_finished_after_the_closing_delimiter() {
    let body = multipart_body("pv", &[("caption", None, b"text")]);
    let mut multipart = reader("pv", body, 1 << 20);

    assert!(multipart.next_part().await.unwrap().is_some());
    assert!(multipart.next_part().await.unwrap().is_none());
    assert!(multipart.next_part().await.unwrap().is_none());
}

#[tokio::test]
async fn multipart_rejects_an_overlong_header_line_and_header_block() {
    let mut line = b"--pv\r\nX:".to_vec();
    line.extend(std::iter::repeat_n(b'a', MAX_HEADER_LINE + 1));
    line.extend_from_slice(b"\r\n\r\n");

    let mut block = b"--pv\r\nX: ".to_vec();
    block.extend(std::iter::repeat_n(b'a', MAX_HEADER_LINE / 2));
    block.extend_from_slice(b"\r\nY: ");
    block.extend(std::iter::repeat_n(b'b', MAX_HEADER_LINE / 2));
    block.extend_from_slice(b"\r\n\r\n");

    for body in [line, block] {
        let mut multipart = reader("pv", body, 1 << 20);
        let error = multipart
            .next_part()
            .await
            .expect_err("header limits are enforced");
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn multipart_rejects_an_overlong_unterminated_header_line() {
    let mut body = b"--pv\r\n".to_vec();
    body.extend(std::iter::repeat_n(b'a', MAX_HEADER_LINE + 1));
    let mut multipart = reader("pv", body, 1 << 20);

    let error = multipart
        .next_part()
        .await
        .expect_err("unterminated header lines have a fixed size limit");

    assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[test]
fn content_length_ignores_invalid_values() {
    let mut headers = HeaderMap::new();
    assert_eq!(content_length(&headers), None);

    headers.insert(header::CONTENT_LENGTH, "123".parse().unwrap());
    assert_eq!(content_length(&headers), Some(123));

    headers.insert(header::CONTENT_LENGTH, "not-a-number".parse().unwrap());
    assert_eq!(content_length(&headers), None);

    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_bytes(b"\xff").unwrap(),
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

#[test]
fn idle_upload_buckets_are_swept_after_the_map_grows_large() {
    let stale = Instant::now() - BUCKET_IDLE;
    {
        let mut buckets = UPLOAD_BUCKETS.lock().unwrap();
        buckets.clear();
        for index in 0..=BUCKET_SWEEP_AT {
            buckets.insert(
                Uuid::from_u128(index as u128 + 1),
                Bucket {
                    tokens: 1.0,
                    updated: stale,
                },
            );
        }
    }

    assert!(take_upload_token(Uuid::from_u128(u128::MAX), 2).is_ok());
    assert_eq!(UPLOAD_BUCKETS.lock().unwrap().len(), 1);
}

#[test]
fn only_a_multipart_content_type_is_accepted() {
    assert_eq!(
        boundary("multipart/form-data; boundary=abc").as_deref(),
        Some("abc")
    );
    assert_eq!(
        boundary("multipart/form-data; charset=utf-8; boundary=\"a b\"").as_deref(),
        Some("a b")
    );
    assert_eq!(boundary("application/json"), None);
    assert_eq!(boundary("multipart/form-data"), None);
    assert_eq!(boundary("multipart/form-data; boundary="), None);
    assert_eq!(
        boundary(&format!("multipart/form-data; boundary={}", "x".repeat(71))),
        None
    );
}

#[test]
fn find_locates_a_needle_only_where_it_is() {
    assert_eq!(find(b"hello world", b"world"), Some(6));
    assert_eq!(find(b"aaab", b"aab"), Some(1));
    assert_eq!(find(b"abc", b"abcd"), None);
    assert_eq!(find(b"", b"a"), None);
    assert_eq!(find(b"abc", b""), None);
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
    let state = test_state_with(
        pool.clone(),
        scratch.path().to_path_buf(),
        crate::photos::worker::VariantQueue::detached(),
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
        crate::photos::worker::VariantQueue { sender },
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
        crate::photos::worker::VariantQueue { sender },
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
        crate::photos::worker::VariantQueue { sender },
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
    let original =
        crate::photos::storage::original_path(scratch.path(), &ContentHash(photo.content_hash));
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
