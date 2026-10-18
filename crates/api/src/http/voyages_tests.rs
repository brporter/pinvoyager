use crate::photo_voyage;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use secrecy::SecretString;
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt as _;

use super::*;
use crate::config::{Config, DatabaseConfig, PhotosConfig, ServerConfig, TelemetryConfig};

/// A state whose pool is lazy and never connects. Enough to prove routing
/// and every check that happens before a query.
fn test_state() -> AppState {
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
        photos: PhotosConfig::default(),
    };

    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("a syntactically valid url makes a lazy pool");

    AppState::new(
        pool,
        Arc::new(config),
        crate::auth::ProviderRegistry::empty(),
        crate::photos::worker::VariantQueue::detached(),
    )
}

fn state_with_pool(pool: PgPool) -> AppState {
    let mut state = test_state();
    state.pool = pool;
    state
}

fn test_user(user_id: Uuid) -> CurrentUser {
    let now = Utc::now();
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

async fn create_photo_for_test(pool: &PgPool, owner_id: Uuid, suffix: u64) -> Uuid {
    crate::db::repo::photos::create(
        pool,
        &crate::db::repo::photos::NewPhoto {
            voyage_id: photo_voyage::fixture_voyage(pool, owner_id).await,
            owner_id,
            content_hash: format!("{suffix:064x}"),
            byte_size: 10,
            width: 2,
            height: 2,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: None,
            longitude: None,
        },
    )
    .await
    .expect("insert photo")
    .id
}

async fn send(method: &str, uri: &str, body: &str) -> Response {
    crate::http::router(test_state())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("valid request"),
        )
        .await
        .expect("router is infallible")
}

async fn status_of(method: &str, uri: &str) -> StatusCode {
    send(method, uri, "{}").await.status()
}

#[sqlx::test]
async fn create_update_and_delete_handlers_apply_owner_changes(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let stranger = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create stranger");
    let state = state_with_pool(pool.clone());

    let (status, Json(created)) = create(
        State(state.clone()),
        test_user(owner.id),
        Json(CreateRequest {
            title: None,
            description: Some("  first draft  ".to_owned()),
            granularity: None,
        }),
    )
    .await
    .expect("create draft");
    assert_eq!(status, StatusCode::CREATED);
    assert!(!created.published);
    assert_eq!(created.title, "");
    assert_eq!(created.description.as_deref(), Some("first draft"));
    assert_eq!(created.granularity, "km1");
    assert_eq!(created.precision_m, Some(500));

    let voyage = repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "A trip".to_owned(),
            description: Some("Keep this".to_owned()),
            granularity: "exact".to_owned(),
        },
    )
    .await
    .expect("create voyage for update");
    let Json(updated) = update(
        State(state.clone()),
        test_user(owner.id),
        Path(voyage.id),
        Json(UpdateRequest {
            title: Some("  Revised trip  ".to_owned()),
            description: Some(None),
            granularity: Some("km10".to_owned()),
            published: Some(true),
        }),
    )
    .await
    .expect("update and publish");
    assert_eq!(updated.title, "Revised trip");
    assert_eq!(updated.description, None);
    assert!(updated.published);
    assert_eq!(updated.granularity, "km10");

    assert!(matches!(
        update(
            State(state.clone()),
            test_user(stranger.id),
            Path(voyage.id),
            Json(UpdateRequest::default())
        )
        .await,
        Err(Rejection::App(AppError::NotFound))
    ));

    assert_eq!(
        destroy(State(state.clone()), test_user(owner.id), Path(voyage.id))
            .await
            .expect("owner can delete voyage"),
        StatusCode::NO_CONTENT
    );
    assert!(matches!(
        destroy(State(state.clone()), test_user(owner.id), Path(voyage.id)).await,
        Err(Rejection::App(AppError::NotFound))
    ));
    assert!(matches!(
        destroy(State(state), test_user(stranger.id), Path(created.id)).await,
        Err(Rejection::App(AppError::NotFound))
    ));
}

#[sqlx::test]
async fn attach_reorder_and_detach_handlers_preserve_photo_ownership(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let stranger = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create stranger");
    let voyage = repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: String::new(),
            description: None,
            granularity: "exact".to_owned(),
        },
    )
    .await
    .expect("create draft voyage");
    let first = create_photo_for_test(&pool, owner.id, 11).await;
    let second = create_photo_for_test(&pool, owner.id, 12).await;
    let third = create_photo_for_test(&pool, owner.id, 13).await;
    let foreign = create_photo_for_test(&pool, stranger.id, 14).await;
    let state = state_with_pool(pool.clone());
    let user = test_user(owner.id);

    repo::move_photos(&pool, voyage.id, owner.id, &[first, second, third])
        .await
        .expect("move fixture photos");
    assert_eq!(
        repo::move_photos(&pool, voyage.id, owner.id, &[foreign])
            .await
            .expect("ownership enforced"),
        0
    );

    let Json(reordered) = reorder(
        State(state.clone()),
        user.clone(),
        Path(voyage.id),
        Json(PhotoIdsRequest {
            photo_ids: vec![third, Uuid::now_v7(), first],
        }),
    )
    .await
    .expect("owner can reorder photos");
    assert_eq!(
        reordered
            .photos
            .iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>(),
        vec![third, first, second]
    );

    assert_eq!(
        detach(State(state.clone()), user.clone(), Path((voyage.id, third)),)
            .await
            .expect("owner can detach their photo"),
        StatusCode::NO_CONTENT
    );
    assert!(matches!(
        detach(State(state.clone()), user, Path((voyage.id, third)),).await,
        Err(Rejection::App(AppError::NotFound))
    ));
}

#[sqlx::test]
async fn voyage_list_uses_owner_ids_and_publication_times_as_cursors(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create owner");
    let older = repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Older draft".to_owned(),
            description: None,
            granularity: "exact".to_owned(),
        },
    )
    .await
    .expect("create older voyage");
    let newer = repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Published trip".to_owned(),
            description: None,
            granularity: "km1".to_owned(),
        },
    )
    .await
    .expect("create newer voyage");
    let published = repo::update(
        &pool,
        newer.id,
        owner.id,
        &VoyagePatch {
            title: None,
            description: None,
            granularity: None,
            published: Some(true),
        },
    )
    .await
    .expect("publish voyage")
    .expect("voyage exists");
    let state = state_with_pool(pool);

    let Json(mine) = list(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Query(ListQuery {
            mine: true,
            before: None,
            limit: Some(1),
        }),
    )
    .await
    .expect("owner listing succeeds");
    assert_eq!(mine.voyages[0].id, newer.id);
    assert_eq!(
        mine.next_before.as_deref(),
        Some(newer.id.to_string().as_str())
    );

    let Json(mine_older) = list(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Query(ListQuery {
            mine: true,
            before: mine.next_before,
            limit: Some(2),
        }),
    )
    .await
    .expect("owner cursor succeeds");
    assert_eq!(mine_older.voyages.len(), 1);
    assert_eq!(mine_older.voyages[0].id, older.id);
    assert_eq!(mine_older.next_before, None);

    let Json(feed) = list(
        State(state.clone()),
        MaybeUser(None),
        Query(ListQuery {
            mine: false,
            before: None,
            limit: Some(1),
        }),
    )
    .await
    .expect("public feed succeeds anonymously");
    assert_eq!(feed.voyages.len(), 1);
    assert_eq!(feed.voyages[0].id, newer.id);
    assert_eq!(
        feed.next_before.as_deref(),
        Some(
            repo::PublishedCursor::of(&published)
                .expect("published cursor")
                .encode()
                .as_str()
        )
    );

    let Json(feed_older) = list(
        State(state.clone()),
        MaybeUser(None),
        Query(ListQuery {
            mine: false,
            before: feed.next_before,
            limit: Some(1),
        }),
    )
    .await
    .expect("public timestamp cursor succeeds");
    assert!(feed_older.voyages.is_empty());
    assert_eq!(feed_older.next_before, None);

    assert!(matches!(
        list(
            State(state.clone()),
            MaybeUser(None),
            Query(ListQuery {
                mine: true,
                before: None,
                limit: None,
            })
        )
        .await,
        Err(Rejection::App(AppError::Unauthorized))
    ));
    assert!(matches!(
        list(
            State(state.clone()),
            MaybeUser(Some(test_user(owner.id))),
            Query(ListQuery {
                mine: true,
                before: Some("bad-id".to_owned()),
                limit: None,
            })
        )
        .await,
        Err(Rejection::App(AppError::BadRequest(_)))
    ));
    assert!(matches!(
        list(
            State(state),
            MaybeUser(None),
            Query(ListQuery {
                mine: false,
                before: Some("bad-time".to_owned()),
                limit: None,
            })
        )
        .await,
        Err(Rejection::App(AppError::BadRequest(_)))
    ));
}

// -- routing ------------------------------------------------------------

/// Every mutating route exists and requires a session. 401 rather than 404
/// is also what proves they are merged into the router at all — an unwired
/// route would 404 here, with no database involved either way.
#[tokio::test]
async fn every_mutating_route_requires_a_session() {
    let id = Uuid::now_v7();
    let photo = Uuid::now_v7();

    for (method, uri) in [
        ("POST", "/api/voyages".to_owned()),
        ("PATCH", format!("/api/voyages/{id}")),
        ("DELETE", format!("/api/voyages/{id}")),
        ("POST", format!("/api/voyages/{id}/photos")),
        ("PATCH", format!("/api/voyages/{id}/photos/order")),
        ("DELETE", format!("/api/voyages/{id}/photos/{photo}")),
    ] {
        assert_eq!(
            status_of(method, &uri).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri}"
        );
    }
}

/// T6 and D6: the read surface is routed, and it takes no session.
///
/// Proved with a method the path does not answer, because a routed path
/// with the wrong method is 405 while an unrouted path is 404. That tells
/// "wired" from "not wired" without entering a handler — which is what
/// makes this assertable at all, since a `GET` would reach the repository.
///
/// The no-session half is a type-level fact rather than an assertion: these
/// three handlers take [`MaybeUser`], whose extraction is `Infallible`, so
/// there is no path through them that can answer 401. Requiring a session
/// would be a change to a signature, not to an `if`.
#[tokio::test]
async fn the_read_surface_is_routed_and_takes_no_session() {
    let id = Uuid::now_v7();

    for uri in [
        "/api/voyages".to_owned(),
        format!("/api/voyages/{id}"),
        format!("/api/voyages/{id}/map"),
    ] {
        assert_eq!(
            status_of("PUT", &uri).await,
            StatusCode::METHOD_NOT_ALLOWED,
            "{uri} is not routed"
        );
    }

    assert_eq!(
        status_of("PUT", "/api/voyagers").await,
        StatusCode::NOT_FOUND,
        "the 405s above only mean something if an unrouted path is a 404"
    );
}

/// `.../photos/order` and `.../photos/{photo}` share a prefix. The literal
/// segment has to win, or a reorder would be read as a detach of a photo
/// whose id is the word "order".
#[tokio::test]
async fn the_order_route_is_not_shadowed_by_the_photo_id_route() {
    let id = Uuid::now_v7();

    // Both reach a handler that demands a session, which is only reachable
    // if the path matched the route with that method on it.
    assert_eq!(
        status_of("PATCH", &format!("/api/voyages/{id}/photos/order")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(
            "DELETE",
            &format!("/api/voyages/{id}/photos/{}", Uuid::now_v7())
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
}

// -- the granularity boundary -------------------------------------------

/// An unrecognized stored precision logs an error and falls back to Km50.
#[test]
fn an_unknown_stored_granularity_fails_closed() {
    assert_eq!(stored_granularity("exact"), Granularity::Exact);
    assert_eq!(stored_granularity("km1"), Granularity::Km1);
    assert_eq!(stored_granularity("km10"), Granularity::Km10);
    assert_eq!(stored_granularity("km50"), Granularity::Km50);

    for unknown in ["", "km100", "EXACT", "km5", "null"] {
        assert_eq!(
            stored_granularity(unknown),
            Granularity::Km50,
            "{unknown:?} must fall back to the coarsest setting, not the finest"
        );
    }
}

/// The labels the design prints. Served rather than derived on the client
/// so that the badge, the card and the photo caption cannot drift apart.
#[test]
fn every_precision_has_the_designed_label() {
    assert_eq!(granularity_label(Granularity::Exact), "Exact");
    assert_eq!(granularity_label(Granularity::Km1), "~1 km");
    assert_eq!(granularity_label(Granularity::Km10), "~10 km");
    assert_eq!(granularity_label(Granularity::Km50), "~50 km");
}

/// Half a coordinate is not half a location. A row with a latitude and no
/// longitude joins the unplaced group rather than becoming a pin on the
/// prime meridian.
#[test]
fn a_photo_missing_one_coordinate_is_unplaced() {
    for (latitude, longitude) in [(Some(43.06), None), (None, Some(141.35)), (None, None)] {
        assert!(place(Granularity::Exact, latitude, longitude).is_none());
    }
}

// -- response shapes -----------------------------------------------------

fn a_photo() -> VoyagePhoto {
    VoyagePhoto {
        photo_id: Uuid::now_v7(),
        position: 0,
        captured_at: None,
        width: 4032,
        height: 3024,
        variant_status: "ready".to_owned(),
        latitude: None,
        longitude: None,
    }
}

fn a_voyage(granularity: &str, published: bool) -> Voyage {
    Voyage {
        id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        title: "Three weeks in Hokkaido".to_owned(),
        description: Some("Late winter, mostly on foot.".to_owned()),
        published,
        granularity: granularity.to_owned(),
        created_at: Utc::now(),
        published_at: published.then(Utc::now),
    }
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// Photo responses carry rounded coordinates without separate raw latitude
/// or longitude fields.
#[test]
fn an_unplaced_photo_is_listed_and_flagged_with_no_bare_coordinates() {
    let voyage = a_voyage("km10", true);
    let response = VoyageResponse::build(voyage, vec![a_photo(), a_photo()]);
    let json = serde_json::to_value(&response).expect("serializes");

    assert_eq!(json["photo_count"], 2);
    assert_eq!(json["placed_count"], 0);
    assert_eq!(json["place_count"], 0);
    assert_eq!(
        json["unplaced_count"], 2,
        "unplaced photos are a normal outcome, not an omission"
    );

    let photos = json["photos"].as_array().expect("photos are listed");
    assert_eq!(photos.len(), 2, "an unplaced photo is still in the voyage");

    assert_eq!(
        keys(&photos[0]),
        vec![
            "captured_at",
            "coordinate",
            "display_url",
            "full_url",
            "height",
            "id",
            "placed",
            "position",
            "thumbnail_url",
            "variant_status",
            "width",
        ],
        "the photo DTO grew or lost a field; if it grew a coordinate one, \
             it must be a `Coordinate` and nothing else"
    );

    assert_eq!(photos[0]["placed"], false);
    assert!(photos[0]["coordinate"].is_null());
}

/// Discovery cards include a precision label without exposing coordinates.
#[test]
fn a_discovery_card_carries_a_precision_label_and_no_location() {
    let json =
        serde_json::to_value(VoyageSummary::from(a_voyage("km10", true))).expect("serializes");

    assert_eq!(
        keys(&json),
        vec![
            "cover_url",
            "created_at",
            "description",
            "granularity",
            "granularity_label",
            "id",
            "owner_id",
            "photos_pending",
            "precision_m",
            "published",
            "published_at",
            "title",
        ],
        "the discovery card grew a field; if it is a location it must be a \
             `Coordinate` built through `place`"
    );

    assert_eq!(json["granularity_label"], "~10 km");
    assert_eq!(json["precision_m"], 5_000);
}

/// Map responses omit photo dimensions, capture times, and processing status.
#[test]
fn the_map_payload_carries_only_what_a_map_draws() {
    let voyage = a_voyage("km10", true);
    let granularity = stored_granularity(&voyage.granularity);
    let photo = a_photo();

    let response = MapResponse {
        voyage_id: voyage.id,
        granularity: granularity.as_str(),
        granularity_label: granularity_label(granularity),
        precision_m: granularity.half_cell_m(),
        pins: Vec::new(),
        unplaced_count: 1,
        unplaced: vec![MapUnplaced {
            photo_id: photo.photo_id,
            thumbnail_url: variant_url(photo.photo_id, VariantKind::Thumb),
            placed: false,
        }],
    };

    let json = serde_json::to_value(&response).expect("serializes");
    assert_eq!(
        keys(&json),
        vec![
            "granularity",
            "granularity_label",
            "pins",
            "precision_m",
            "unplaced",
            "unplaced_count",
            "voyage_id",
        ]
    );
    assert_eq!(
        keys(&json["unplaced"][0]),
        vec!["photo_id", "placed", "thumbnail_url"],
        "a photograph the map cannot draw is still accounted for, and flagged"
    );

    // Detail responses additionally include photo presentation and processing data.
    let detail =
        serde_json::to_value(VoyageResponse::build(voyage, vec![photo])).expect("serializes");
    let detail_photo = detail["photos"][0].as_object().expect("an object");

    for absent_from_a_pin in ["width", "height", "captured_at", "variant_status"] {
        assert!(
            detail_photo.contains_key(absent_from_a_pin),
            "the detail payload should carry {absent_from_a_pin}"
        );
    }
}

// -- validation ----------------------------------------------------------

/// The design's only publish precondition, and it must arrive as a
/// field-level error so the edit screen can put it beside the input.
#[tokio::test]
async fn publishing_without_a_title_is_a_field_error_on_the_title() {
    for blank in ["", "   ", "\t\n"] {
        let rejection = publishable(blank, true).expect_err("a title is required");
        let (status, body) = rendered(rejection).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], "validation_failed");
        assert_eq!(body["errors"][0]["field"], "title");
        assert_eq!(body["errors"][0]["message"], TITLE_REQUIRED);
    }
}

/// A draft with no title is a legal, ordinary state — it is what the edit
/// screen exists to fill in.
#[test]
fn a_draft_needs_no_title() {
    assert!(publishable("", false).is_ok());
    assert!(publishable("Hokkaido", true).is_ok());
}

#[test]
fn titles_and_descriptions_are_trimmed_and_bounded() {
    assert_eq!(
        clean_title(Some("  Hokkaido  ")).expect("valid"),
        "Hokkaido"
    );
    assert_eq!(clean_title(None).expect("valid"), "");

    // Characters, not bytes: a limit that rejects a Japanese title a third
    // the length of an English one is a bug.
    let long_ascii = "a".repeat(MAX_TITLE_LEN);
    assert!(clean_title(Some(&long_ascii)).is_ok());
    assert!(clean_title(Some(&"a".repeat(MAX_TITLE_LEN + 1))).is_err());
    assert!(clean_title(Some(&"京".repeat(MAX_TITLE_LEN))).is_ok());

    assert_eq!(clean_description(Some("   ")).expect("valid"), None);
    assert_eq!(clean_description(None).expect("valid"), None);
    assert_eq!(
        clean_description(Some(" snow ")).expect("valid").as_deref(),
        Some("snow")
    );
    assert!(clean_description(Some(&"a".repeat(MAX_DESCRIPTION_LEN + 1))).is_err());
}

#[test]
fn sqlx_rejections_are_preserved_as_database_errors() {
    assert!(matches!(
        Rejection::from(sqlx::Error::PoolClosed),
        Rejection::App(AppError::Database(sqlx::Error::PoolClosed))
    ));
}

#[tokio::test]
async fn an_unknown_precision_is_a_field_error_on_the_granularity() {
    assert_eq!(parse_granularity("km10").expect("known"), Granularity::Km10);

    let (status, body) = rendered(parse_granularity("km7").expect_err("unknown")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["field"], "granularity");
}

/// Duplicate photo IDs are removed while retaining their first position.
#[test]
fn photo_ids_are_deduplicated_in_order_and_bounded() {
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();

    assert_eq!(
        clean_photo_ids(vec![a, b, a]).expect("valid"),
        vec![a, b],
        "duplicates are dropped and the first occurrence keeps its place"
    );

    assert!(clean_photo_ids(Vec::new()).is_err());
    assert!(clean_photo_ids(vec![a; MAX_PHOTO_IDS + 1]).is_err());
}

// -- request parsing -----------------------------------------------------

/// Absent, `null` and a value are three different intentions, and a PATCH
/// that could not tell the first two apart would make "remove my
/// description" impossible to express.
#[test]
fn a_patch_tells_an_absent_description_from_an_explicit_null() {
    let absent: UpdateRequest = serde_json::from_str("{}").expect("valid");
    assert_eq!(absent.description, None);

    let cleared: UpdateRequest = serde_json::from_str(r#"{"description":null}"#).expect("valid");
    assert_eq!(cleared.description, Some(None));

    let set: UpdateRequest = serde_json::from_str(r#"{"description":"snow"}"#).expect("valid");
    assert_eq!(set.description, Some(Some("snow".to_owned())));
}

/// A cursor is opaque, so a malformed one is the caller's mistake and gets
/// a 400 — not a 500 from a parse deeper in.
#[tokio::test]
async fn a_malformed_cursor_is_a_400() {
    let response = send("GET", "/api/voyages?before=yesterday", "").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = send("GET", "/api/voyages?mine=true&before=nope", "").await;
    // No session, so authorization is answered first — which is itself the
    // right order.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Drafts are in the `mine` listing, so that mode needs a session even
/// though the feed beside it does not.
#[tokio::test]
async fn listing_your_own_voyages_requires_a_session() {
    assert_eq!(
        status_of("GET", "/api/voyages?mine=true").await,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn a_variant_url_is_built_from_an_id_and_a_closed_set_of_names() {
    let id = Uuid::now_v7();
    assert_eq!(
        variant_url(id, VariantKind::Thumb),
        format!("/api/photos/{id}/thumb")
    );
    assert_eq!(
        variant_url(id, VariantKind::Full),
        format!("/api/photos/{id}/full")
    );
}

/// The status and the parsed body of a rejection, so a test asserts on what
/// a client actually receives rather than on the enum that produced it.
async fn rendered(rejection: Rejection) -> (StatusCode, Value) {
    let response = rejection.into_response();
    let status = response.status();

    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .expect("body is small");

    (
        status,
        serde_json::from_slice(&body).expect("the body is json"),
    )
}

/// Public response builders must all respect a coarse voyage precision.
#[test]
fn no_endpoint_emits_the_exact_coordinate_for_a_coarse_voyage() {
    let latitude = 43.062096;
    let longitude = 141.354376;

    let photo = VoyagePhoto {
        latitude: Some(latitude),
        longitude: Some(longitude),
        ..a_photo()
    };

    for spelling in ["km1", "km10", "km50"] {
        let voyage = a_voyage(spelling, true);
        let granularity = stored_granularity(spelling);

        let detail =
            serde_json::to_string(&VoyageResponse::build(voyage.clone(), vec![photo.clone()]))
                .expect("serializes");

        let map = serde_json::to_string(&MapResponse {
            voyage_id: voyage.id,
            granularity: granularity.as_str(),
            granularity_label: granularity_label(granularity),
            precision_m: granularity.half_cell_m(),
            pins: vec![MapPin {
                photo_id: photo.photo_id,
                coordinate: place(granularity, photo.latitude, photo.longitude).expect("placed"),
                thumbnail_url: variant_url(photo.photo_id, VariantKind::Thumb),
            }],
            unplaced_count: 0,
            unplaced: Vec::new(),
        })
        .expect("serializes");

        let summary = serde_json::to_string(&VoyageSummary::from(voyage)).expect("serializes");

        for (name, body) in [("detail", detail), ("map", map), ("summary", summary)] {
            for exact in [latitude.to_string(), longitude.to_string()] {
                assert!(
                    !body.contains(&exact),
                    "{spelling} {name} response leaked {exact}: {body}"
                );
            }
        }
    }
}

/// T5's other half: at `exact` the full precision is returned, so the
/// setting is a choice rather than a one-way door.
#[test]
fn an_exact_voyage_returns_full_precision() {
    let photo = VoyagePhoto {
        latitude: Some(43.062096),
        longitude: Some(141.354376),
        ..a_photo()
    };

    let json = serde_json::to_value(VoyageResponse::build(a_voyage("exact", true), vec![photo]))
        .expect("serializes");

    assert_eq!(json["photos"][0]["placed"], true);
    assert_eq!(json["photos"][0]["coordinate"]["latitude"], 43.062096);
    assert_eq!(json["photos"][0]["coordinate"]["longitude"], 141.354376);
    assert!(
        json["photos"][0]["coordinate"]["precision_m"].is_null(),
        "an exact coordinate claims no radius"
    );
}

/// T5. Reading the same photograph twice returns identical coordinates.
/// Anything resampled per request makes pins jitter between loads and, far
/// worse, lets repeated observation average the noise away.
#[test]
fn reading_the_same_photo_twice_returns_the_same_point() {
    let photo = VoyagePhoto {
        latitude: Some(43.062096),
        longitude: Some(141.354376),
        ..a_photo()
    };

    let once = serde_json::to_string(&VoyageResponse::build(
        a_voyage("km10", true),
        vec![photo.clone()],
    ))
    .expect("serializes");
    let twice = serde_json::to_string(&VoyageResponse::build(a_voyage("km10", true), vec![photo]))
        .expect("serializes");

    // The ids differ between the two voyages, so compare the part that must
    // not move.
    let once: Value = serde_json::from_str(&once).expect("json");
    let twice: Value = serde_json::from_str(&twice).expect("json");
    assert_eq!(
        once["photos"][0]["coordinate"],
        twice["photos"][0]["coordinate"]
    );
}

/// The "12 places" beside "48 photos" is counted after rounding, so a
/// coarse voyage does not report how many distinct spots exist inside one
/// cell.
#[test]
fn places_are_counted_after_rounding() {
    // Two points a few hundred meters apart: distinct at exact precision,
    // one place at ~50 km.
    let near = |lat: f64, lon: f64| VoyagePhoto {
        latitude: Some(lat),
        longitude: Some(lon),
        ..a_photo()
    };
    let photos = vec![near(43.0620, 141.3543), near(43.0631, 141.3560)];

    let coarse = VoyageResponse::build(a_voyage("km50", true), photos.clone());
    assert_eq!(coarse.placed_count, 2);
    assert_eq!(
        coarse.place_count, 1,
        "two photographs in one cell are one place"
    );

    let exact = VoyageResponse::build(a_voyage("exact", true), photos);
    assert_eq!(exact.place_count, 2);
}

#[sqlx::test]
async fn deleting_a_voyage_deletes_photos_but_preserves_shared_storage(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage_id = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let second_voyage = photo_voyage::fixture_voyage(&pool, owner.id).await;
    let unique = create_photo_for_test(&pool, owner.id, 101).await;
    let shared = create_photo_for_test(&pool, owner.id, 102).await;
    let survivor = create_photo_for_test(&pool, owner.id, 102).await;
    repo::move_photos(&pool, voyage_id, owner.id, &[unique, shared])
        .await
        .expect("photos");
    repo::move_photos(&pool, second_voyage, owner.id, &[survivor])
        .await
        .expect("shared record");
    let scratch = crate::photos::decode::testutil::ScratchDir::new("voyage-delete-content");
    let mut state = state_with_pool(pool.clone());
    Arc::get_mut(&mut state.config)
        .expect("unique config")
        .photos
        .storage_root = scratch.path().to_path_buf();
    let unique_path = crate::photos::storage::original_path(
        scratch.path(),
        &crate::photos::storage::ContentHash::try_from(format!("{:064x}", 101))
            .expect("valid fixture hash"),
    );
    let shared_path = crate::photos::storage::original_path(
        scratch.path(),
        &crate::photos::storage::ContentHash::try_from(format!("{:064x}", 102))
            .expect("valid fixture hash"),
    );
    for path in [&unique_path, &shared_path] {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("content directory");
        std::fs::write(path, b"fixture").expect("content");
    }
    assert_eq!(
        destroy(State(state.clone()), test_user(owner.id), Path(voyage_id))
            .await
            .expect("delete"),
        StatusCode::NO_CONTENT
    );
    assert!(
        crate::db::repo::photos::find_by_id(&pool, unique)
            .await
            .expect("photo")
            .is_none()
    );
    assert!(
        crate::db::repo::photos::find_by_id(&pool, shared)
            .await
            .expect("photo")
            .is_none()
    );
    assert!(
        crate::db::repo::photos::find_by_id(&pool, survivor)
            .await
            .expect("photo")
            .is_some()
    );
    assert!(
        unique_path.exists(),
        "HTTP deletion leaves bytes queued for maintenance"
    );
    crate::photos::cleanup::sweep(&pool, scratch.path())
        .await
        .expect("background cleanup");
    assert!(!unique_path.exists());
    assert!(shared_path.exists());
    destroy(State(state), test_user(owner.id), Path(second_voyage))
        .await
        .expect("delete last reference");
    assert!(
        shared_path.exists(),
        "last reference also queues background removal"
    );
    crate::photos::cleanup::sweep(&pool, scratch.path())
        .await
        .expect("background cleanup");
    assert!(!shared_path.exists());
}

#[sqlx::test(migrations = "./migrations")]
async fn voyage_list_covers_select_ready_photos_in_order_and_report_pending_work(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Cover trip".into(),
            description: None,
            granularity: "exact".into(),
        },
    )
    .await
    .expect("voyage");
    let first = create_photo_for_test(&pool, owner.id, 9101).await;
    let second = create_photo_for_test(&pool, owner.id, 9102).await;
    let third = create_photo_for_test(&pool, owner.id, 9103).await;
    repo::move_photos(&pool, voyage.id, owner.id, &[first, second, third])
        .await
        .expect("move");
    repo::reorder(&pool, voyage.id, owner.id, &[first, second, third])
        .await
        .expect("order");
    crate::db::repo::photos::set_variant_status(&pool, second, "ready")
        .await
        .expect("ready");
    crate::db::repo::photos::set_variant_status(&pool, third, "failed")
        .await
        .expect("failed");
    let state = state_with_pool(pool.clone());
    let query = || ListQuery {
        mine: true,
        before: None,
        limit: Some(100),
    };
    let Json(response) = list(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Query(query()),
    )
    .await
    .expect("list");
    let json = serde_json::to_value(response).expect("json");
    let cover = json["voyages"]
        .as_array()
        .expect("voyages")
        .iter()
        .find(|v| v["id"] == voyage.id.to_string())
        .expect("trip");
    assert_eq!(cover["cover_url"], variant_url(second, VariantKind::Thumb));
    assert_eq!(cover["photos_pending"], true);
    let empty = json["voyages"]
        .as_array()
        .expect("voyages")
        .iter()
        .find(|v| v["id"] != voyage.id.to_string())
        .expect("empty fixture voyage");
    assert!(empty["cover_url"].is_null());
    assert_eq!(empty["photos_pending"], false);
    crate::db::repo::photos::set_variant_status(&pool, first, "ready")
        .await
        .expect("ready");
    let Json(response) = list(
        State(state.clone()),
        MaybeUser(Some(test_user(owner.id))),
        Query(query()),
    )
    .await
    .expect("list");
    let json = serde_json::to_value(response).expect("json");
    let cover = json["voyages"]
        .as_array()
        .expect("voyages")
        .iter()
        .find(|v| v["id"] == voyage.id.to_string())
        .expect("trip");
    assert_eq!(cover["cover_url"], variant_url(first, VariantKind::Thumb));
    assert_eq!(cover["photos_pending"], false);
    repo::update(
        &pool,
        voyage.id,
        owner.id,
        &VoyagePatch {
            published: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("publish");
    let Json(response) = list(
        State(state),
        MaybeUser(None),
        Query(ListQuery {
            mine: false,
            before: None,
            limit: Some(100),
        }),
    )
    .await
    .expect("public list");
    assert_eq!(response.voyages.len(), 1);
    let json = serde_json::to_value(response).expect("json");
    assert_eq!(
        json["voyages"][0]["cover_url"],
        variant_url(first, VariantKind::Thumb)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn voyage_list_does_not_offer_pending_or_failed_photos_as_covers(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let id = create_photo_for_test(&pool, owner.id, 9201).await;
    let state = state_with_pool(pool.clone());
    for (status, pending) in [("pending", true), ("failed", false)] {
        crate::db::repo::photos::set_variant_status(&pool, id, status)
            .await
            .expect("status");
        let Json(response) = list(
            State(state.clone()),
            MaybeUser(Some(test_user(owner.id))),
            Query(ListQuery {
                mine: true,
                before: None,
                limit: Some(100),
            }),
        )
        .await
        .expect("list");
        let json = serde_json::to_value(response).expect("json");
        assert!(json["voyages"][0]["cover_url"].is_null());
        assert_eq!(json["voyages"][0]["photos_pending"], pending);
    }
}

#[sqlx::test]
async fn public_feed_visits_every_voyage_with_a_tied_timestamp(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    for _ in 0..5 {
        sqlx::query("INSERT INTO voyages (id, owner_id, title, published, published_at) VALUES ($1,$2,'Trip',true,'2026-01-01T00:00:00Z')")
            .bind(Uuid::now_v7()).bind(owner.id).execute(&pool).await.expect("voyage");
    }
    let state = state_with_pool(pool);
    let mut cursor = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let Json(page) = list(
            State(state.clone()),
            MaybeUser(None),
            Query(ListQuery {
                mine: false,
                before: cursor,
                limit: Some(2),
            }),
        )
        .await
        .expect("page");
        for voyage in page.voyages {
            assert!(seen.insert(voyage.id));
        }
        cursor = page.next_before;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 5);
}

#[sqlx::test]
async fn publication_checks_the_title_after_waiting_for_concurrent_edits(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = repo::create(
        &pool,
        &NewVoyage {
            owner_id: owner.id,
            title: "Before concurrent edit".to_owned(),
            description: None,
            granularity: "exact".to_owned(),
        },
    )
    .await
    .expect("voyage");
    // Hold an uncommitted title edit. A publisher must wait, then validate the
    // committed title; reading first would validate stale data and publish blank.
    let mut editing = pool.begin().await.expect("editing transaction");
    sqlx::query("UPDATE voyages SET title='' WHERE id=$1")
        .bind(voyage.id)
        .execute(&mut *editing)
        .await
        .expect("clear title");
    let state = state_with_pool(pool.clone());
    let publisher = tokio::spawn(async move {
        update(
            State(state),
            test_user(owner.id),
            Path(voyage.id),
            Json(UpdateRequest {
                published: Some(true),
                ..UpdateRequest::default()
            }),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%voyages%')")
                .fetch_one(&pool).await.expect("publication waiting for lock");
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("publisher reached the locked row");
    editing.commit().await.expect("commit concurrent edit");
    let error = publisher
        .await
        .expect("publisher task")
        .expect_err("blank title cannot be published");
    assert_eq!(
        error.into_response().status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let saved = repo::find_by_id(&pool, voyage.id)
        .await
        .expect("read")
        .expect("voyage remains");
    assert!(saved.title.is_empty());
    assert!(!saved.published);
}

#[sqlx::test]
async fn suspension_after_authentication_prevents_voyage_deletion(pool: PgPool) {
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let voyage = crate::photo_voyage::fixture_voyage(&pool, owner.id).await;
    let user = test_user(owner.id);
    sqlx::query("UPDATE users SET suspended=true WHERE id=$1")
        .bind(owner.id)
        .execute(&pool)
        .await
        .expect("suspend");
    assert!(matches!(
        destroy(State(state_with_pool(pool.clone())), user, Path(voyage)).await,
        Err(Rejection::App(AppError::Forbidden))
    ));
    assert!(
        repo::find_by_id(&pool, voyage)
            .await
            .expect("lookup")
            .is_some()
    );
}
