//! Regression tests for the startup recovery sweep (phase 4, T7).
//!
//! A process that dies mid-batch leaves rows at `variant_status = 'pending'`.
//! Nothing will ever enqueue them again — the only thing that did was the
//! upload request, which returned long ago — so without a startup sweep they
//! sit in the owner's library forever, visible and never viewable.
//!
//! Found by hand-testing: a photo set back to `pending` was NOT picked up on
//! restart. These tests isolate which layer is at fault.

use secrecy::SecretString;
use sqlx::PgPool;
use uuid::Uuid;

use pinvoyager_api::config::{
    AuthConfig, Config, DatabaseConfig, PhotosConfig, ServerConfig, TelemetryConfig,
};
use pinvoyager_api::db::repo::photos::{self, NewPhoto};
use pinvoyager_api::db::repo::users;
use pinvoyager_api::photos::storage::{self, ContentHash};
use pinvoyager_api::photos::worker::{self, VariantJob};

struct ScratchRoot(std::path::PathBuf);

impl ScratchRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pv-worker-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&path).expect("create scratch storage root");
        Self(path)
    }
}

impl Drop for ScratchRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn worker_config(root: &ScratchRoot) -> Config {
    Config {
        server: ServerConfig::default(),
        database: DatabaseConfig {
            url: SecretString::from("postgres://unused"),
            max_connections: 1,
            min_connections: 1,
            acquire_timeout_secs: 5,
        },
        telemetry: TelemetryConfig::default(),
        auth: AuthConfig {
            base_url: "http://127.0.0.1:8080".parse().expect("valid base URL"),
            session: Default::default(),
            providers: Default::default(),
            dev_login: false,
        },
        photos: PhotosConfig {
            storage_root: root.0.clone(),
            ..PhotosConfig::default()
        },
    }
}

async fn a_pending_photo(pool: &PgPool) -> Uuid {
    let user = users::create(pool, Some("Owner"), None)
        .await
        .expect("create user");

    let photo = photos::create(
        pool,
        &NewPhoto {
            owner_id: user.id,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            byte_size: 1234,
            width: 800,
            height: 600,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: None,
            longitude: None,
        },
    )
    .await
    .expect("create photo");

    // `create` leaves the row at the column default, which is what an upload
    // does. Asserted rather than assumed, because the whole sweep hangs off
    // this value.
    assert_eq!(
        photo.variant_status, "pending",
        "a freshly created photo must start pending, or nothing is ever queued"
    );

    photo.id
}

/// The repository layer, in isolation. If this passes and the sweep still does
/// not fire, the fault is in the worker's wiring rather than the query.
#[sqlx::test]
async fn pending_finds_a_photo_awaiting_variants(pool: PgPool) {
    let id = a_pending_photo(&pool).await;

    let found = photos::pending(&pool, 1_000).await.expect("query pending");

    assert!(
        found.contains(&id),
        "photos::pending must return a row whose variant_status is 'pending'; got {found:?}"
    );
}

/// A photo that has already been processed must not be swept up again —
/// otherwise every restart re-encodes the entire library.
#[sqlx::test]
async fn pending_ignores_photos_that_are_already_ready(pool: PgPool) {
    let id = a_pending_photo(&pool).await;

    photos::set_variant_status(&pool, id, "ready")
        .await
        .expect("mark ready");

    let found = photos::pending(&pool, 1_000).await.expect("query pending");

    assert!(
        !found.contains(&id),
        "a ready photo must not be requeued; a restart would otherwise re-encode everything"
    );
}

/// A photo that failed must not be retried forever either. A file that will not
/// decode will not decode on the hundredth restart, and an infinite retry on a
/// poisoned row is how a queue stops draining.
#[sqlx::test]
async fn pending_ignores_photos_that_failed(pool: PgPool) {
    let id = a_pending_photo(&pool).await;

    photos::set_variant_status(&pool, id, "failed")
        .await
        .expect("mark failed");

    let found = photos::pending(&pool, 1_000).await.expect("query pending");

    assert!(!found.contains(&id), "a failed photo must not be requeued");
}

#[sqlx::test]
async fn processing_a_deleted_photo_is_a_successful_noop(pool: PgPool) {
    let scratch = ScratchRoot::new();
    let config = worker_config(&scratch);

    worker::process(
        &pool,
        &config,
        &VariantJob {
            photo_id: Uuid::now_v7(),
        },
    )
    .await
    .expect("a photo deleted before processing is an ordinary no-op");
}

#[sqlx::test]
async fn processing_an_already_ready_photo_does_not_rewrite_variants(pool: PgPool) {
    let id = a_pending_photo(&pool).await;
    photos::set_variant_status(&pool, id, "ready")
        .await
        .expect("mark photo ready");
    let scratch = ScratchRoot::new();
    let config = worker_config(&scratch);

    worker::process(&pool, &config, &VariantJob { photo_id: id })
        .await
        .expect("an already processed photo is a no-op");

    assert!(std::fs::read_dir(&scratch.0).unwrap().next().is_none());
}

#[sqlx::test]
async fn processing_a_valid_pending_photo_writes_variants_and_marks_it_ready(pool: PgPool) {
    let id = a_pending_photo(&pool).await;
    let photo = photos::find_by_id(&pool, id)
        .await
        .expect("read photo")
        .expect("photo exists");
    let scratch = ScratchRoot::new();
    let hash = ContentHash(photo.content_hash);
    let original = storage::original_path(&scratch.0, &hash);
    std::fs::create_dir_all(original.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(40, 32, image::Rgb([20, 40, 60]))
        .save_with_format(&original, image::ImageFormat::Jpeg)
        .expect("write original JPEG");
    let config = worker_config(&scratch);

    worker::process(&pool, &config, &VariantJob { photo_id: id })
        .await
        .expect("valid pending photo should be processed");

    let updated = photos::find_by_id(&pool, id)
        .await
        .expect("read processed photo")
        .expect("photo remains in the library");
    assert_eq!(updated.variant_status, "ready");
    for kind in pinvoyager_api::photos::VariantKind::ALL {
        assert!(storage::variant_path(&scratch.0, &hash, kind).is_file());
    }
}

#[sqlx::test]
async fn processing_a_corrupt_original_marks_the_photo_failed(pool: PgPool) {
    let id = a_pending_photo(&pool).await;
    let scratch = ScratchRoot::new();
    let config = worker_config(&scratch);

    let error = worker::process(&pool, &config, &VariantJob { photo_id: id })
        .await
        .expect_err("an undecodable original must fail variant generation");

    assert!(matches!(
        error,
        pinvoyager_api::error::AppError::Internal(_)
    ));
    let updated = photos::find_by_id(&pool, id)
        .await
        .expect("read failed photo")
        .expect("photo remains in the library");
    assert_eq!(updated.variant_status, "failed");
}
