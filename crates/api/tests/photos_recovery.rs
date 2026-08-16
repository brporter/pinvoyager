//! Regression tests for the startup recovery sweep (phase 4, T7).
//!
//! A process that dies mid-batch leaves rows at `variant_status = 'pending'`.
//! Nothing will ever enqueue them again — the only thing that did was the
//! upload request, which returned long ago — so without a startup sweep they
//! sit in the owner's library forever, visible and never viewable.
//!
//! Found by hand-testing: a photo set back to `pending` was NOT picked up on
//! restart. These tests isolate which layer is at fault.

use sqlx::PgPool;
use uuid::Uuid;

use pinvoyager_api::db::repo::photos::{self, NewPhoto};
use pinvoyager_api::db::repo::users;

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
