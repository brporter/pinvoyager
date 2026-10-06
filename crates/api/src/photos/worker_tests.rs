use std::sync::Arc;
use std::time::Duration;
use std::{fs, path::Path};

use super::*;
use crate::config::{AuthConfig, DatabaseConfig, PhotosConfig, ServerConfig, TelemetryConfig};
use crate::db::repo::{photos as photo_repo, users};
use crate::photos::decode::testutil::{ScratchDir, asymmetric_image};
use secrecy::SecretString;
use sqlx::{PgPool, postgres::PgPoolOptions};

/// The mapping the worker uses to pick a decoder. A value the column can
/// hold but this cannot resolve would make every photo of that format fail.
#[test]
fn every_stored_format_resolves() {
    for format in [
        ImageFormat::Jpeg,
        ImageFormat::Png,
        ImageFormat::WebP,
        ImageFormat::Heic,
    ] {
        let parsed = parse_format(format.as_str()).expect("round trips");
        assert_eq!(parsed, format);
    }
}

#[test]
fn an_unknown_format_is_refused_rather_than_guessed() {
    assert!(parse_format("gif").is_err());
    assert!(parse_format("").is_err());
}

/// The statuses have to match the migration's CHECK constraint exactly; a
/// mismatch is a constraint violation on a row nobody is watching.
#[test]
fn statuses_match_the_schema() {
    assert_eq!(STATUS_PENDING, "pending");
    assert_eq!(STATUS_READY, "ready");
    assert_eq!(STATUS_FAILED, "failed");
}

/// T7: a full queue makes the producer wait. It does not grow, and it does
/// not drop the job on the floor.
#[tokio::test]
async fn a_full_queue_applies_backpressure() {
    let (sender, mut receiver) = mpsc::channel(1);
    let queue = VariantQueue { sender };

    queue
        .enqueue(VariantJob {
            photo_id: Uuid::now_v7(),
        })
        .await
        .expect("first job fits");

    // The second send cannot complete until something is taken off.
    let blocked = tokio::time::timeout(
        Duration::from_millis(100),
        queue.enqueue(VariantJob {
            photo_id: Uuid::now_v7(),
        }),
    )
    .await;
    assert!(blocked.is_err(), "a full queue accepted a second job");

    // ...and it completes once there is room, rather than having failed.
    receiver.recv().await.expect("first job is still there");
    tokio::time::timeout(
        Duration::from_secs(5),
        queue.enqueue(VariantJob {
            photo_id: Uuid::now_v7(),
        }),
    )
    .await
    .expect("space became available")
    .expect("the worker is still there");
}

/// A detached queue refuses work rather than swallowing it, so a test that
/// accidentally depends on variant generation fails loudly.
#[tokio::test]
async fn a_detached_queue_refuses_work() {
    let queue = VariantQueue::detached();
    assert!(
        queue
            .enqueue(VariantJob {
                photo_id: Uuid::now_v7(),
            })
            .await
            .is_err()
    );
}

#[test]
fn derive_writes_all_variants_from_one_original() {
    let scratch = ScratchDir::new("worker-derive");
    let root = scratch.path();
    let hash = ContentHash("ab".repeat(32));
    let original = storage::original_path(root, &hash);
    fs::create_dir_all(original.parent().expect("sharded path has a parent")).unwrap();
    asymmetric_image(32, 24)
        .save_with_format(&original, image::ImageFormat::Jpeg)
        .expect("write original fixture");

    derive(root, &hash, ImageFormat::Jpeg, 80.0).expect("derive variants");

    for kind in VariantKind::ALL {
        let path = storage::variant_path(root, &hash, kind);
        assert!(path.is_file(), "missing {kind:?} variant at {path:?}");
        assert!(fs::metadata(path).unwrap().len() > 0);
    }
}

#[test]
fn derive_returns_an_error_when_the_original_cannot_be_decoded() {
    let scratch = ScratchDir::new("worker-bad-original");
    let root = scratch.path();
    let hash = ContentHash("cd".repeat(32));
    let original = storage::original_path(root, &hash);
    fs::create_dir_all(original.parent().expect("sharded path has a parent")).unwrap();
    fs::write(&original, b"not an image").unwrap();

    let error = derive(root, &hash, ImageFormat::Jpeg, 80.0).unwrap_err();

    assert!(matches!(error, AppError::Internal(_)));
}

#[test]
fn atomic_write_creates_directories_and_replaces_the_destination() {
    let scratch = ScratchDir::new("worker-atomic-write");
    let destination = scratch.path().join("nested/variant.webp");

    write_atomic(&destination, b"first").expect("first atomic write");
    write_atomic(&destination, b"replacement").expect("replace existing variant");

    assert_eq!(fs::read(destination).unwrap(), b"replacement");
}

#[test]
fn atomic_write_reports_when_the_parent_cannot_be_created() {
    let scratch = ScratchDir::new("worker-atomic-write-parent-error");
    let blocker = scratch.path().join("file");
    fs::write(&blocker, b"not a directory").unwrap();
    let destination = blocker.join("variant.webp");

    let error = write_atomic(&destination, b"content").unwrap_err();

    assert!(matches!(error, AppError::Internal(_)));
}

#[test]
fn atomic_write_rejects_a_path_without_a_parent() {
    let error = write_atomic(Path::new("/"), b"content").unwrap_err();

    assert!(matches!(error, AppError::Internal(_)));
}

fn worker_config(root: &Path) -> Config {
    Config {
        server: ServerConfig::default(),
        database: DatabaseConfig {
            url: SecretString::from("postgres://unused"),
            max_connections: 1,
            min_connections: 0,
            acquire_timeout_secs: 1,
        },
        telemetry: TelemetryConfig::default(),
        auth: AuthConfig {
            base_url: "http://127.0.0.1:8080".parse().expect("valid base URL"),
            session: Default::default(),
            providers: Default::default(),
            dev_login: false,
        },
        photos: PhotosConfig {
            storage_root: root.to_path_buf(),
            ..PhotosConfig::default()
        },
    }
}

#[tokio::test]
async fn recovery_reports_database_failures_and_leaves_the_queue_untouched() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(20))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("valid lazy pool URL");
    let (sender, mut receiver) = mpsc::channel(1);

    recover(pool, sender).await;

    assert!(receiver.try_recv().is_err());
}

#[sqlx::test]
async fn recovery_returns_cleanly_when_no_photos_are_pending(pool: PgPool) {
    let (sender, mut receiver) = mpsc::channel(1);

    recover(pool, sender).await;

    assert!(receiver.try_recv().is_err());
}

async fn insert_pending_photo(pool: &PgPool) -> Uuid {
    let user = users::create(pool, Some("Recovery owner"), None)
        .await
        .expect("create owner");
    photo_repo::create(
        pool,
        &photo_repo::NewPhoto {
            owner_id: user.id,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            byte_size: 1,
            width: 1,
            height: 1,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: None,
            longitude: None,
        },
    )
    .await
    .expect("create pending photo")
    .id
}

#[sqlx::test]
async fn recovery_enqueues_each_photo_that_is_still_pending(pool: PgPool) {
    let id = insert_pending_photo(&pool).await;
    let (sender, mut receiver) = mpsc::channel(1);

    recover(pool, sender).await;

    assert_eq!(receiver.recv().await.map(|job| job.photo_id), Some(id));
}

#[sqlx::test]
async fn recovery_stops_when_the_worker_queue_is_closed(pool: PgPool) {
    insert_pending_photo(&pool).await;
    let (sender, receiver) = mpsc::channel(1);
    drop(receiver);

    recover(pool, sender).await;
}

#[tokio::test]
async fn dispatch_stops_when_its_semaphore_is_closed() {
    let (_sender, receiver) = mpsc::channel(1);
    let permits = Arc::new(Semaphore::new(1));
    permits.close();

    dispatch(
        PgPoolOptions::new()
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .expect("valid lazy pool URL"),
        Arc::new(worker_config(Path::new("unused"))),
        receiver,
        permits,
    )
    .await;
}

#[tokio::test]
async fn dispatch_stops_when_every_queue_sender_is_dropped() {
    let (sender, receiver) = mpsc::channel(1);
    drop(sender);

    dispatch(
        PgPoolOptions::new()
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .expect("valid lazy pool URL"),
        Arc::new(worker_config(Path::new("unused"))),
        receiver,
        Arc::new(Semaphore::new(1)),
    )
    .await;
}

#[sqlx::test]
async fn dispatch_processes_queued_jobs_and_releases_their_permits(pool: PgPool) {
    let photo_id = insert_pending_photo(&pool).await;
    let scratch = ScratchDir::new("worker-dispatch");
    let config = Arc::new(worker_config(scratch.path()));
    let (sender, receiver) = mpsc::channel(1);
    sender
        .send(VariantJob { photo_id })
        .await
        .expect("queue the pending photo");
    drop(sender);
    let permits = Arc::new(Semaphore::new(1));

    dispatch(pool.clone(), config, receiver, permits.clone()).await;

    let _permit = tokio::time::timeout(Duration::from_secs(5), permits.acquire())
        .await
        .expect("worker finishes the queued job")
        .expect("worker does not close the semaphore");
    let photo = photo_repo::find_by_id(&pool, photo_id)
        .await
        .expect("read worker result")
        .expect("photo remains present");
    assert_eq!(photo.variant_status, STATUS_FAILED);
}
