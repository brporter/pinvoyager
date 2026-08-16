//! Public visibility of photograph bytes (D6).
//!
//! Every voyage is public, which has to mean the *pictures* are public and not
//! merely the JSON describing them. Phase 4 served variants to their owner
//! only, so a published voyage rendered as a page of broken images for everyone
//! but its author — the voyage was world-readable and its photographs were not.
//!
//! These tests pin the three-way distinction that fixes it:
//!
//! * attached to a **published** voyage → anyone may fetch the bytes
//! * attached only to a **draft** → nobody but the owner, or "draft" would leak
//!   its contents to anyone holding a photo id
//! * attached to **nothing** → nobody but the owner
//!
//! They drive the real router, because the thing under test is an
//! authorization decision made by a handler, not a predicate in isolation.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use secrecy::SecretString;
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::config::{
    AuthConfig, Config, DatabaseConfig, PhotosConfig, ServerConfig, TelemetryConfig,
};
use pinvoyager_api::db::repo::photos::{self, NewPhoto};
use pinvoyager_api::db::repo::users;
use pinvoyager_api::db::repo::voyages::{self as voyages_repo, NewVoyage, VoyagePatch};
use pinvoyager_api::photos::VariantKind;
use pinvoyager_api::photos::storage::{self, ContentHash};
use pinvoyager_api::photos::worker::VariantQueue;
use pinvoyager_api::state::AppState;

/// A scratch storage root that cleans up after itself, so a failing test does
/// not leave variant files behind for the next run to trip over.
struct ScratchRoot(std::path::PathBuf);

impl ScratchRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pv-pub-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&path).expect("create scratch root");
        Self(path)
    }
}

impl Drop for ScratchRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state(pool: PgPool, root: &ScratchRoot) -> AppState {
    let config = Config {
        server: ServerConfig::default(),
        database: DatabaseConfig {
            url: SecretString::from("postgres://unused".to_owned()),
            max_connections: 1,
            min_connections: 1,
            acquire_timeout_secs: 5,
        },
        telemetry: TelemetryConfig::default(),
        auth: AuthConfig {
            base_url: "http://127.0.0.1:8080".parse().expect("valid url"),
            session: Default::default(),
            providers: Default::default(),
            dev_login: false,
        },
        photos: PhotosConfig {
            storage_root: root.0.clone(),
            ..PhotosConfig::default()
        },
    };

    AppState::new(
        pool,
        Arc::new(config),
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    )
}

/// A photo whose variant bytes actually exist on disk.
///
/// Writing the file matters: without it every request 404s for a reason that
/// has nothing to do with authorization, and the test would "pass" while
/// proving nothing.
async fn photo_with_variant(pool: &PgPool, root: &ScratchRoot, owner: Uuid) -> Uuid {
    let hash = format!("{:064x}", Uuid::now_v7().as_u128());

    let photo = photos::create(
        pool,
        &NewPhoto {
            owner_id: owner,
            content_hash: hash.clone(),
            byte_size: 32,
            width: 100,
            height: 80,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: None,
            longitude: None,
        },
    )
    .await
    .expect("create photo");

    photos::set_variant_status(pool, photo.id, "ready")
        .await
        .expect("mark ready");

    let path = storage::variant_path(&root.0, &ContentHash(hash), VariantKind::Thumb);
    std::fs::create_dir_all(path.parent().expect("variant path has a parent"))
        .expect("create shard dirs");
    // Not a real WebP. The handler serves bytes and sets a server-determined
    // Content-Type; it never decodes, so any bytes exercise the same path.
    std::fs::write(&path, b"not-really-webp-but-served-verbatim").expect("write variant");

    photo.id
}

async fn fetch_thumb(state: &AppState, photo_id: Uuid) -> StatusCode {
    let response = pinvoyager_api::http::router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/api/photos/{photo_id}/thumb"))
                .body(Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("router responds");

    response.status()
}

#[sqlx::test]
async fn an_unattached_photo_is_not_public(pool: PgPool) {
    let root = ScratchRoot::new();
    let state = state(pool.clone(), &root);

    let owner = users::create(&pool, Some("Owner"), None)
        .await
        .expect("create user");
    let photo = photo_with_variant(&pool, &root, owner.id).await;

    assert_eq!(
        fetch_thumb(&state, photo).await,
        StatusCode::NOT_FOUND,
        "a photo in nobody's voyage must not be fetchable by an anonymous caller"
    );
}

#[sqlx::test]
async fn a_draft_does_not_make_its_photos_public(pool: PgPool) {
    let root = ScratchRoot::new();
    let state = state(pool.clone(), &root);

    let owner = users::create(&pool, Some("Owner"), None)
        .await
        .expect("create user");
    let photo = photo_with_variant(&pool, &root, owner.id).await;

    let voyage = voyages_repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Unpublished".to_owned(),
            description: None,
            granularity: "exact".to_owned(),
        },
    )
    .await
    .expect("create voyage");

    voyages_repo::attach_photos(&pool, voyage.id, owner.id, &[photo])
        .await
        .expect("attach");

    // The whole point of a draft. If attaching to an unpublished voyage granted
    // visibility, "draft" would leak its contents to anyone holding a photo id.
    assert_eq!(
        fetch_thumb(&state, photo).await,
        StatusCode::NOT_FOUND,
        "a draft must not make its photographs fetchable"
    );
}

#[sqlx::test]
async fn publishing_a_voyage_makes_its_photos_fetchable_by_anyone(pool: PgPool) {
    let root = ScratchRoot::new();
    let state = state(pool.clone(), &root);

    let owner = users::create(&pool, Some("Owner"), None)
        .await
        .expect("create user");
    let photo = photo_with_variant(&pool, &root, owner.id).await;

    let voyage = voyages_repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Published".to_owned(),
            description: None,
            granularity: "km10".to_owned(),
        },
    )
    .await
    .expect("create voyage");

    voyages_repo::attach_photos(&pool, voyage.id, owner.id, &[photo])
        .await
        .expect("attach");

    // Draft first, so the transition is what the test observes rather than the
    // end state alone — a handler that ignored `published` entirely would pass
    // an end-state-only assertion.
    assert_eq!(
        fetch_thumb(&state, photo).await,
        StatusCode::NOT_FOUND,
        "still a draft at this point"
    );

    voyages_repo::update(
        &pool,
        voyage.id,
        owner.id,
        &VoyagePatch {
            published: Some(true),
            ..VoyagePatch::default()
        },
    )
    .await
    .expect("publish");

    assert_eq!(
        fetch_thumb(&state, photo).await,
        StatusCode::OK,
        "publishing must make the photographs fetchable, or the voyage is public \
         in name only and renders as broken images for everyone but its author"
    );
}

/// Unpublishing has to take the pictures back down again, or "unpublish" is
/// only a change to a listing.
#[sqlx::test]
async fn unpublishing_withdraws_public_access(pool: PgPool) {
    let root = ScratchRoot::new();
    let state = state(pool.clone(), &root);

    let owner = users::create(&pool, Some("Owner"), None)
        .await
        .expect("create user");
    let photo = photo_with_variant(&pool, &root, owner.id).await;

    let voyage = voyages_repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Temporarily public".to_owned(),
            description: None,
            granularity: "exact".to_owned(),
        },
    )
    .await
    .expect("create voyage");

    voyages_repo::attach_photos(&pool, voyage.id, owner.id, &[photo])
        .await
        .expect("attach");
    voyages_repo::update(
        &pool,
        voyage.id,
        owner.id,
        &VoyagePatch {
            published: Some(true),
            ..VoyagePatch::default()
        },
    )
    .await
    .expect("publish");
    assert_eq!(fetch_thumb(&state, photo).await, StatusCode::OK);

    voyages_repo::update(
        &pool,
        voyage.id,
        owner.id,
        &VoyagePatch {
            published: Some(false),
            ..VoyagePatch::default()
        },
    )
    .await
    .expect("unpublish");

    assert_eq!(
        fetch_thumb(&state, photo).await,
        StatusCode::NOT_FOUND,
        "unpublishing must withdraw access to the bytes, not just to the listing"
    );
}
