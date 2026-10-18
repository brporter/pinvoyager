//! Discovery-map tests through the production router.
//!
//! Checks publication filtering, per-voyage rounding before clustering, viewport
//! validation, and marker grouping. Each test uses an isolated migrated database
//! so other fixtures cannot appear in its discovery response.

#[path = "support/photo_voyage.rs"]
mod photo_voyage;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use secrecy::SecretString;
use serde_json::Value;
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
use pinvoyager_api::photos::worker::VariantQueue;
use pinvoyager_api::state::AppState;
use pinvoyager_api::voyages::Granularity;

// ---------------------------------------------------------------------------
// The coordinates under test
// ---------------------------------------------------------------------------

/// Sapporo, roughly, to six decimal places — exactly what `numeric(9, 6)`
/// stores, so the value round-trips without the database rewriting the digits.
///
/// Deliberately not a round number. `43.0` is a plausible cell center and would
/// appear in a correctly rounded response by coincidence, so a sweep for it
/// would fail against a perfectly good implementation.
const EXACT_LAT: f64 = 43.061_234;
const EXACT_LON: f64 = 141.354_321;

/// Five-decimal coordinate prefix used to detect exact or truncated values
/// in these coarse-response fixtures.
const LAT_PREFIX: &str = "43.06123";
const LON_PREFIX: &str = "141.35432";

/// A rectangle comfortably around Sapporo.
const AROUND_SAPPORO: &str = "west=140.0&south=42.0&east=142.5&north=44.0";

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn state(pool: PgPool) -> AppState {
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
            initial_admins: Default::default(),
            base_url: "http://127.0.0.1:8080".parse().expect("valid url"),
            session: Default::default(),
            providers: Default::default(),
            dev_login: false,
        },
        photos: PhotosConfig::default(),
    };

    AppState::new(
        pool,
        Arc::new(config),
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    )
}

struct Reply {
    status: StatusCode,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("response is not JSON ({err}): {}", self.body))
    }

    fn clusters(&self) -> Vec<Value> {
        self.json()["clusters"]
            .as_array()
            .expect("the response carries a clusters array")
            .clone()
    }
}

/// Drives the **production router**, middleware and all.
///
/// A hand-assembled subset would keep passing if `http::router` forgot to merge
/// the voyage routes, which is exactly the kind of omission a new endpoint
/// invites.
async fn map(pool: &PgPool, query: &str) -> Reply {
    let response = pinvoyager_api::http::router(state(pool.clone()))
        .oneshot(
            Request::builder()
                .uri(format!("/api/map?{query}"))
                .body(Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("the router is infallible");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body is readable");

    Reply {
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn a_user(pool: &PgPool) -> Uuid {
    users::create(pool, None, None)
        .await
        .expect("create user")
        .id
}

async fn photo_at(pool: &PgPool, owner: Uuid, latitude: f64, longitude: f64) -> Uuid {
    photos::create(
        pool,
        &NewPhoto {
            voyage_id: photo_voyage::fixture_voyage(pool, owner).await,
            owner_id: owner,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            byte_size: 4096,
            width: 4032,
            height: 3024,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
        },
    )
    .await
    .expect("create photo")
    .id
}

async fn voyage_with(
    pool: &PgPool,
    owner: Uuid,
    granularity: Granularity,
    published: bool,
    photo_ids: &[Uuid],
) -> Uuid {
    let voyage = voyages_repo::create(
        pool,
        &NewVoyage {
            owner_id: owner,
            title: "Hokkaido".to_owned(),
            description: None,
            granularity: granularity.as_str().to_owned(),
        },
    )
    .await
    .expect("create voyage");

    if !photo_ids.is_empty() {
        voyages_repo::move_photos(pool, voyage.id, owner, photo_ids)
            .await
            .expect("attach photos");
    }

    if published {
        voyages_repo::update(
            pool,
            voyage.id,
            owner,
            &VoyagePatch {
                published: Some(true),
                ..VoyagePatch::default()
            },
        )
        .await
        .expect("publish");
    }

    voyage.id
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// The endpoint takes no session and reads none. Every voyage it draws is
/// published, so the answer is the same for everyone.
#[sqlx::test]
async fn anyone_may_read_the_map(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    voyage_with(&pool, owner, Granularity::Exact, true, &[photo]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.clusters().len(), 1);
}

/// Draft photos are omitted from the anonymous discovery map.
#[sqlx::test]
async fn a_draft_is_not_on_the_map(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    voyage_with(&pool, owner, Granularity::Exact, false, &[photo]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply.clusters().is_empty(),
        "an unpublished voyage must not appear on the public map: {}",
        reply.body
    );
}

/// Unpublishing has to take it back down, or "unpublish" is only a change to a
/// listing.
#[sqlx::test]
async fn unpublishing_removes_a_voyage_from_the_map(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    let voyage = voyage_with(&pool, owner, Granularity::Exact, true, &[photo]).await;

    assert_eq!(
        map(&pool, &format!("{AROUND_SAPPORO}&zoom=10"))
            .await
            .clusters()
            .len(),
        1
    );

    voyages_repo::update(
        &pool,
        voyage,
        owner,
        &VoyagePatch {
            published: Some(false),
            ..VoyagePatch::default()
        },
    )
    .await
    .expect("unpublish");

    assert!(
        map(&pool, &format!("{AROUND_SAPPORO}&zoom=10"))
            .await
            .clusters()
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// The granularity boundary
// ---------------------------------------------------------------------------

/// The leak sweep, on the response that mixes voyages.
///
/// Searches the characters that actually went on the wire, not a parsed float:
/// `43.061234` and `43.0612339999` are the same number to a comparison against
/// an epsilon and are the same leak to a reader.
#[sqlx::test]
async fn a_coarse_voyage_never_puts_its_exact_coordinate_on_the_map(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    voyage_with(&pool, owner, Granularity::Km10, true, &[photo]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;

    // Paired with a check that something was returned. An implementation that
    // dropped every coordinate, or answered 404 everywhere, would pass the
    // grep perfectly and be useless.
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.clusters().len(), 1, "the photograph should be drawn");

    assert!(
        !reply.body.contains(LAT_PREFIX),
        "the exact latitude reached the discovery map: {}",
        reply.body
    );
    assert!(
        !reply.body.contains(LON_PREFIX),
        "the exact longitude reached the discovery map: {}",
        reply.body
    );
}

/// And the same photograph in an *exact* voyage does publish its true position,
/// which is what shows the sweep above is detecting rounding rather than the
/// endpoint being broken.
#[sqlx::test]
async fn an_exact_voyage_publishes_the_position_it_recorded(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    voyage_with(&pool, owner, Granularity::Exact, true, &[photo]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;

    assert!(reply.body.contains(LAT_PREFIX), "{}", reply.body);
    assert!(reply.body.contains(LON_PREFIX), "{}", reply.body);
}

/// Each voyage is rounded to its **own** setting, not to one setting for the
/// response. Two voyages holding the same photograph at different precisions
/// must produce two different published positions.
#[sqlx::test]
async fn each_voyage_is_rounded_to_its_own_precision(pool: PgPool) {
    let owner = a_user(&pool).await;

    let precise = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    voyage_with(&pool, owner, Granularity::Exact, true, &[precise]).await;

    // Far enough away to be its own marker at this zoom.
    let vague = photo_at(&pool, owner, 43.5, 141.9).await;
    voyage_with(&pool, owner, Granularity::Km50, true, &[vague]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;
    let clusters = reply.clusters();

    assert_eq!(clusters.len(), 2, "{}", reply.body);

    let precisions: Vec<Option<u64>> = clusters
        .iter()
        .map(|cluster| cluster["coordinate"]["precision_m"].as_u64())
        .collect();

    // The exact one carries no precision at all — the field is absent, not
    // null — and the coarse one carries half a 50 km cell.
    assert!(
        precisions.contains(&None),
        "the exact voyage's marker should carry no precision: {}",
        reply.body
    );
    assert!(
        precisions.contains(&Some(25_000)),
        "the ~50 km voyage's marker should carry a 25 km radius: {}",
        reply.body
    );
}

// ---------------------------------------------------------------------------
// Clustering
// ---------------------------------------------------------------------------

/// Two photographs a few hundred meters apart, drawn as one marker when the
/// camera is far out and as two when it is close in. This is the behavior the
/// endpoint exists for.
#[sqlx::test]
async fn nearby_photographs_merge_at_a_low_zoom_and_separate_at_a_high_one(pool: PgPool) {
    let owner = a_user(&pool).await;

    // ~220 m north and ~290 m east of the first.
    let first = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    let second = photo_at(&pool, owner, 43.063_210, 141.357_890).await;
    voyage_with(&pool, owner, Granularity::Exact, true, &[first, second]).await;

    let far = map(&pool, &format!("{AROUND_SAPPORO}&zoom=8")).await;
    assert_eq!(far.clusters().len(), 1, "{}", far.body);
    assert_eq!(far.clusters()[0]["count"].as_u64(), Some(2));

    let close = map(&pool, &format!("{AROUND_SAPPORO}&zoom=16")).await;
    assert_eq!(close.clusters().len(), 2, "{}", close.body);
    for cluster in close.clusters() {
        assert_eq!(cluster["count"].as_u64(), Some(1));
    }
}

/// A marker stands for a real photograph, and says which voyage to go to. A
/// count with nothing behind it would be a marker a reader cannot follow.
#[sqlx::test]
async fn a_marker_names_a_photograph_and_its_voyage(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    let voyage = voyage_with(&pool, owner, Granularity::Exact, true, &[photo]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;
    let cluster = reply.clusters().remove(0);

    assert_eq!(
        cluster["photo_id"].as_str(),
        Some(photo.to_string().as_str())
    );
    assert_eq!(
        cluster["voyage_id"].as_str(),
        Some(voyage.to_string().as_str())
    );
    assert_eq!(
        cluster["thumbnail_url"].as_str(),
        Some(format!("/api/photos/{photo}/thumb").as_str())
    );
}

/// Deterministic: the same rectangle asked for twice draws the same markers.
/// Without an ordering the representative of a group would be whichever row the
/// planner happened to return first, and markers would move between reloads for
/// no reason a reader could see.
#[sqlx::test]
async fn the_same_rectangle_answers_the_same_way_twice(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photos: Vec<Uuid> = {
        let mut ids = Vec::new();
        for step in 0..6 {
            ids.push(photo_at(&pool, owner, 43.0 + f64::from(step) * 0.001, 141.3).await);
        }
        ids
    };
    voyage_with(&pool, owner, Granularity::Exact, true, &photos).await;

    let first = map(&pool, &format!("{AROUND_SAPPORO}&zoom=9")).await;
    let second = map(&pool, &format!("{AROUND_SAPPORO}&zoom=9")).await;

    assert_eq!(first.body, second.body);
}

// ---------------------------------------------------------------------------
// The rectangle
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn photographs_outside_the_rectangle_are_left_out(pool: PgPool) {
    let owner = a_user(&pool).await;
    let sapporo = photo_at(&pool, owner, EXACT_LAT, EXACT_LON).await;
    let lisbon = photo_at(&pool, owner, 38.722_252, -9.139_337).await;
    voyage_with(&pool, owner, Granularity::Exact, true, &[sapporo, lisbon]).await;

    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=10")).await;

    assert_eq!(reply.clusters().len(), 1, "{}", reply.body);
    assert_eq!(
        reply.clusters()[0]["photo_id"].as_str(),
        Some(sapporo.to_string().as_str())
    );
}

/// A rectangle from Fiji to Samoa runs 177 to -172: west is numerically greater
/// than east. Read as an ordinary range that is an empty rectangle, and the
/// Pacific would be blank however many photographs were in it.
#[sqlx::test]
async fn a_rectangle_crossing_the_antimeridian_finds_both_sides(pool: PgPool) {
    let owner = a_user(&pool).await;

    let west_of_the_line = photo_at(&pool, owner, -17.713_371, 178.065_032).await; // Fiji
    let east_of_the_line = photo_at(&pool, owner, -13.759_029, -172.104_629).await; // Samoa
    voyage_with(
        &pool,
        owner,
        Granularity::Exact,
        true,
        &[west_of_the_line, east_of_the_line],
    )
    .await;

    let reply = map(
        &pool,
        "west=175.0&south=-20.0&east=-170.0&north=-10.0&zoom=6",
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.clusters().len(),
        2,
        "both sides of the antimeridian should be found: {}",
        reply.body
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// A rectangle whose south edge is north of its north edge is not a rectangle,
/// and answering "no photographs" for it would hide a client bug behind an
/// empty map.
#[sqlx::test]
async fn an_upside_down_rectangle_is_refused(pool: PgPool) {
    let reply = map(&pool, "west=140.0&south=44.0&east=142.0&north=42.0&zoom=10").await;

    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
}

#[sqlx::test]
async fn a_latitude_off_the_globe_is_refused(pool: PgPool) {
    let reply = map(
        &pool,
        "west=140.0&south=-91.0&east=142.0&north=44.0&zoom=10",
    )
    .await;

    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
}

/// NaN has to be refused, and it is easy to leave a hole for it: every
/// comparison against NaN is false, so `south < -90.0 || south > 90.0` says
/// "in range" for it. What the value then projects to is a pixel coordinate
/// with no grid cell.
///
/// This asserts the behavior, not which line produces it — the range checks and
/// the explicit `is_finite` guard both reject it as the code stands.
#[sqlx::test]
async fn a_coordinate_that_is_not_a_number_is_refused(pool: PgPool) {
    let reply = map(&pool, "west=NaN&south=42.0&east=142.0&north=44.0&zoom=10").await;

    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
}

#[sqlx::test]
async fn an_impossible_zoom_is_refused(pool: PgPool) {
    for zoom in ["-1", "99"] {
        let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom={zoom}")).await;

        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "zoom {zoom} should be refused: {}",
            reply.body
        );
    }
}

/// A missing parameter is a client that has not been written yet, not a request
/// for the whole world.
#[sqlx::test]
async fn a_rectangle_with_a_missing_edge_is_refused(pool: PgPool) {
    let reply = map(&pool, "west=140.0&south=42.0&north=44.0&zoom=10").await;

    assert!(
        reply.status.is_client_error(),
        "expected a refusal, got {}: {}",
        reply.status,
        reply.body
    );
}

#[sqlx::test]
async fn discovery_membership_depends_only_on_public_coordinates(pool: PgPool) {
    let owner = a_user(&pool).await;
    let photo = photo_at(&pool, owner, 0.01, 0.01).await;
    voyage_with(&pool, owner, Granularity::Km50, true, &[photo]).await;
    let query = "west=-1&east=1&south=-0.4&north=-0.05&zoom=10";
    let before = map(&pool, query).await.json();
    sqlx::query("UPDATE photos SET latitude=0.4 WHERE id=$1")
        .bind(photo)
        .execute(&pool)
        .await
        .expect("move within the same public cell");
    let after = map(&pool, query).await.json();
    assert_eq!(
        before, after,
        "hidden movement must not change public membership"
    );
    assert!(after["clusters"].as_array().expect("clusters").is_empty());
}

#[sqlx::test]
async fn discovery_pages_candidates_and_reports_truncation_with_distinct_destinations(
    pool: PgPool,
) {
    let owner = a_user(&pool).await;
    let voyage = voyage_with(&pool, owner, Granularity::Km10, true, &[]).await;
    sqlx::query("INSERT INTO photos(id,owner_id,voyage_id,content_hash,byte_size,width,height,original_format,latitude,longitude) SELECT gen_random_uuid(),$1,$2,repeat('a',64),1,1,1,'jpeg',43.061234,141.354321 FROM generate_series(1,20001)")
        .bind(owner).bind(voyage).execute(&pool).await.expect("bulk photos");
    let reply = map(&pool, &format!("{AROUND_SAPPORO}&zoom=0")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let json = reply.json();
    assert_eq!(json["truncated"], true);
    let clusters = json["clusters"].as_array().expect("clusters");
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0]["count"], 20000);
    let destinations = clusters[0]["voyages"].as_array().expect("destinations");
    assert_eq!(
        destinations.len(),
        1,
        "many photos must not duplicate voyage choices"
    );
    assert_eq!(destinations[0]["id"], voyage.to_string());
    assert_eq!(destinations[0]["title"], "Hokkaido");
}
