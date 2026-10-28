//! Compare real router responses with committed frontend fixtures, normalizing
//! generated IDs and timestamps. Ordinary test runs never rewrite files.
//! Regenerate intentionally with PINVOYAGER_UPDATE_FIXTURES=1, then review the
//! JSON changes and run the frontend contract test.

#[path = "support/photo_voyage.rs"]
mod photo_voyage;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tower::ServiceExt as _;
use uuid::Uuid;

use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::config::Config;
use pinvoyager_api::db::repo::photos::{self, NewPhoto};
use pinvoyager_api::photos::worker::{VariantJob, VariantQueue};
use pinvoyager_api::state::AppState;

const DEFAULT_URL: &str = "postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager";

/// Sapporo, roughly, at the six decimal places the column stores. Distinctive
/// enough that a rounded value cannot coincide with it.
const LAT: f64 = 43.061_234;
const LON: f64 = 141.354_321;

/// A few hundred meters from the first, so that a `km10` voyage collapses the
/// two into one place and `place_count` differs from `placed_count` in the
/// captured fixture rather than being coincidentally equal.
const NEAR_LAT: f64 = 43.063_210;
const NEAR_LON: f64 = 141.357_890;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("PINVOYAGER__DATABASE__URL"))
        .unwrap_or_else(|_| DEFAULT_URL.to_owned())
}

// ---------------------------------------------------------------------------
// Where the fixtures go
// ---------------------------------------------------------------------------

#[path = "support/contract_fixtures.rs"]
mod contracts;
use contracts::{fixtures_dir, write_fixture};

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

/// A storage root for this run, removed when the test ends.
struct ScratchRoot(PathBuf);

impl ScratchRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pv-fixtures-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&path).expect("create scratch root");
        Self(path)
    }
}

impl Drop for ScratchRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Built through `Deserialize` rather than by constructing the struct, so a
/// change to the configuration shape breaks this the way it breaks a
/// deployment.
fn test_state(pool: PgPool, root: &ScratchRoot, variants: VariantQueue) -> AppState {
    let config: Config = serde_json::from_value(json!({
        "server": {
            "bind": "127.0.0.1:0",
            "shutdown_timeout_secs": 5,
            "request_timeout_secs": 60,
        },
        "database": {
            "url": database_url(),
            "max_connections": 5,
            "min_connections": 1,
            "acquire_timeout_secs": 5,
        },
        "telemetry": { "log_level": "warn", "json": false },
        "auth": {
            "base_url": "http://127.0.0.1:8080/",
            // Enable debug-only session creation without contacting external providers.
            "dev_login": true,
            "session": { "cookie_secure": false },
        },
        "photos": { "storage_root": root.0.clone() },
    }))
    .expect("the test configuration should deserialize");

    AppState::new(pool, Arc::new(config), ProviderRegistry::empty(), variants)
}

/// One captured response.
struct Reply {
    status: StatusCode,
    request_id: Option<String>,
    /// The `name=value` pair from `Set-Cookie`, ready to send back. Only the
    /// dev login sets one.
    session_cookie: Option<String>,
    body: String,
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router is infallible");

    let status = response.status();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };

    let request_id = header("x-request-id");
    let session_cookie = header("set-cookie").map(|value| {
        // Just the pair; the attributes are for a browser.
        value
            .split(';')
            .next()
            .expect("a cookie header has at least one segment")
            .to_owned()
    });

    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("response body should be readable");

    Reply {
        status,
        request_id,
        session_cookie,
        body: String::from_utf8(bytes.to_vec()).expect("every JSON body here is UTF-8"),
    }
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }

    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("valid request");

    send(app, request).await
}

/// Asserts the expected HTTP status before checking or updating its JSON fixture.
#[track_caller]
fn capture(name: &str, expected: StatusCode, reply: &Reply) {
    assert_eq!(
        reply.status, expected,
        "{name}: expected {expected}, got {} — body: {}",
        reply.status, reply.body
    );
    write_fixture(name, &reply.body);
}

// ---------------------------------------------------------------------------
// Callers
// ---------------------------------------------------------------------------

struct Caller {
    user_id: Uuid,
    cookie: String,
}

async fn sign_in(app: &Router, subject: &str) -> Caller {
    let subject = format!("{subject}-{}", Uuid::now_v7());

    let reply = call(
        app,
        "POST",
        "/auth/dev/login",
        None,
        Some(json!({ "subject": subject, "display_name": "Ada Lovelace" })),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "the dev login must succeed or no fixture below has a session: {}",
        reply.body
    );

    let cookie = reply
        .session_cookie
        .clone()
        .expect("the dev login should set a session cookie");

    let body: Value = serde_json::from_str(&reply.body).expect("the dev login answers with JSON");
    let user_id = body["user_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the dev login reports the user id");

    Caller { user_id, cookie }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// A photo row with variants marked ready, so the URLs in a voyage response
/// point at something and `variant_status` is not `pending` everywhere.
async fn photo(
    pool: &PgPool,
    owner: Uuid,
    at: Option<(f64, f64)>,
    captured: bool,
    ready: bool,
) -> Uuid {
    let photo = photos::create(
        pool,
        &NewPhoto {
            voyage_id: photo_voyage::fixture_voyage(pool, owner).await,
            owner_id: owner,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            byte_size: 3_145_728,
            width: 4032,
            height: 3024,
            original_format: "jpeg".to_owned(),
            captured_at: captured.then(|| {
                chrono::DateTime::parse_from_rfc3339("2025-02-14T09:31:07Z")
                    .expect("a literal timestamp")
                    .into()
            }),
            latitude: at.map(|(lat, _)| lat),
            longitude: at.map(|(_, lon)| lon),
        },
    )
    .await
    .expect("create photo");

    if ready {
        photos::set_variant_status(pool, photo.id, "ready")
            .await
            .expect("mark ready");
    }

    photo.id
}

/// A small real PNG, encoded rather than hand-rolled.
///
/// The upload endpoint sniffs magic bytes and probes the dimensions from the
/// header before anything is stored, so a placeholder byte string would be
/// rejected as "not an image" and there would be no upload fixture to capture.
fn png_bytes() -> Vec<u8> {
    let image = image::RgbImage::from_fn(64, 48, |x, y| {
        image::Rgb([(x * 4) as u8, (y * 5) as u8, 128])
    });

    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encoding a PNG in memory should succeed");

    out.into_inner()
}

/// A `multipart/form-data` body with one file part.
fn multipart(boundary: &str, filename: &str, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"files\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

// ---------------------------------------------------------------------------
// The capture
// ---------------------------------------------------------------------------

/// Exercises JSON endpoints in one isolated database so fixtures share
/// consistent account, voyage, and photo relationships. Compares responses by
/// default; the update flag regenerates committed fixtures.
#[sqlx::test]
async fn every_endpoint_matches_its_fixture(pool: PgPool) {
    let root = ScratchRoot::new();

    // A real queue with a live receiver. `VariantQueue::detached()` fails on
    // enqueue by design, and the upload handler turns that into a 503 — so the
    // upload fixture cannot be captured without something on the other end.
    // Nothing consumes the jobs; the rows stay `pending`, which is the state
    // an upload response reports anyway.
    let (sender, _variant_jobs) = mpsc::channel::<VariantJob>(16);
    let app = pinvoyager_api::http::router(test_state(
        pool.clone(),
        &root,
        VariantQueue::from_sender(sender),
    ));

    let owner = sign_in(&app, "fixtures-owner").await;
    let cookie = Some(owner.cookie.as_str());
    let saved = call(
        &app,
        "PATCH",
        "/api/me",
        cookie,
        Some(json!({"display_name":"fixture-owner"})),
    )
    .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT);

    // -- session ------------------------------------------------------------

    let me = call(&app, "GET", "/api/me", cookie, None).await;
    assert!(
        me.request_id.is_some(),
        "responses must carry x-request-id; the error screen asks the user to quote it"
    );
    capture("me", StatusCode::OK, &me);

    // -- voyage-owned photos --------------------------------------------------------

    // With coordinates, with a capture time, variants ready.
    let placed = photo(&pool, owner.user_id, Some((LAT, LON)), true, true).await;
    // Nearby: at ~10 km these two are one place, which is what makes
    // `place_count` differ from `placed_count` in the captured voyage.
    let nearby = photo(&pool, owner.user_id, Some((NEAR_LAT, NEAR_LON)), true, true).await;
    // No coordinates and no capture time — an ordinary photograph, not an edge
    // case, and the one that pins the `null`s in the TypeScript types.
    let unplaced = photo(&pool, owner.user_id, None, false, true).await;

    let photo_voyage: Uuid = sqlx::query_scalar("SELECT voyage_id FROM photos WHERE id=$1")
        .bind(placed)
        .fetch_one(&pool)
        .await
        .expect("photo voyage");
    pinvoyager_api::db::repo::voyages::move_photos(
        &pool,
        photo_voyage,
        owner.user_id,
        &[nearby, unplaced],
    )
    .await
    .expect("fixture voyage photos");

    // A genuine upload, through the multipart reader, the magic-byte sniff, the
    // dimension probe and the storage commit. This is the only fixture whose
    // photo row this test did not write itself.
    let boundary = "----pinvoyagerfixtureboundary";
    let upload = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/voyages/{}/photos",
                photo_voyage::fixture_voyage(&pool, owner.user_id).await
            ))
            .header("cookie", &owner.cookie)
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(multipart(boundary, "shore.png", &png_bytes())))
            .expect("valid request"),
    )
    .await;
    capture("photo_upload", StatusCode::CREATED, &upload);

    capture(
        "photo",
        StatusCode::OK,
        &call(&app, "GET", &format!("/api/photos/{placed}"), cookie, None).await,
    );
    capture(
        "photo_list",
        StatusCode::OK,
        &call(
            &app,
            "GET",
            &format!(
                "/api/voyages/{}/photos?limit=50",
                sqlx::query_scalar::<_, Uuid>("SELECT voyage_id FROM photos WHERE id=$1")
                    .bind(placed)
                    .fetch_one(&pool)
                    .await
                    .expect("voyage")
            ),
            cookie,
            None,
        )
        .await,
    );

    // -- voyages ------------------------------------------------------------

    let created = call(
        &app,
        "POST",
        "/api/voyages",
        cookie,
        Some(json!({
            "title": "Three weeks in Hokkaido",
            "description": "Late winter, mostly on foot.",
            "granularity": "km10",
        })),
    )
    .await;
    capture("voyage_created", StatusCode::CREATED, &created);

    let voyage_id = serde_json::from_str::<Value>(&created.body).expect("json")["id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the create response names the voyage");

    pinvoyager_api::db::repo::voyages::move_photos(
        &pool,
        voyage_id,
        owner.user_id,
        &[placed, nearby, unplaced],
    )
    .await
    .expect("fixture photos");
    capture(
        "voyage_photos",
        StatusCode::OK,
        &call(
            &app,
            "GET",
            &format!("/api/voyages/{voyage_id}"),
            cookie,
            None,
        )
        .await,
    );

    capture(
        "voyage_reordered",
        StatusCode::OK,
        &call(
            &app,
            "PATCH",
            &format!("/api/voyages/{voyage_id}/photos/order"),
            cookie,
            Some(json!({ "photo_ids": [unplaced, placed, nearby] })),
        )
        .await,
    );

    capture(
        "voyage_updated",
        StatusCode::OK,
        &call(
            &app,
            "PATCH",
            &format!("/api/voyages/{voyage_id}"),
            cookie,
            Some(json!({ "description": "Late winter, mostly on foot.", "published": true })),
        )
        .await,
    );

    // A draft, so `?mine=true` returns something the public feed does not.
    let draft = call(
        &app,
        "POST",
        "/api/voyages",
        cookie,
        Some(json!({ "title": "Sanriku coast, unfinished", "granularity": "exact" })),
    )
    .await;
    assert_eq!(draft.status, StatusCode::CREATED, "{}", draft.body);
    let draft_id = serde_json::from_str::<Value>(&draft.body).expect("json")["id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the create response names the voyage");

    // Read as an anonymous caller: the published voyage is public, and reading
    // it without a session is the request the discovery flow actually makes.
    let detail = call(
        &app,
        "GET",
        &format!("/api/voyages/{voyage_id}"),
        None,
        None,
    )
    .await;
    capture("voyage_detail", StatusCode::OK, &detail);

    let body: Value = serde_json::from_str(&detail.body).expect("json");
    assert_eq!(body["photo_count"], 3);
    assert_eq!(body["placed_count"], 2);
    assert_eq!(
        body["place_count"], 1,
        "the two nearby photographs should collapse into one place at ~10 km, or the fixture \
         does not exercise the difference between the two counts"
    );
    assert_eq!(body["unplaced_count"], 1);
    assert!(
        body["photos"]
            .as_array()
            .expect("photos")
            .iter()
            .any(|photo| photo["coordinate"].is_null()),
        "no photograph in the fixture has a null coordinate, so the TypeScript check would \
         never see that `coordinate` is nullable"
    );

    capture(
        "voyage_map",
        StatusCode::OK,
        &call(
            &app,
            "GET",
            &format!("/api/voyages/{voyage_id}/map"),
            None,
            None,
        )
        .await,
    );

    // The discovery map, over every published voyage rather than one of them.
    // Read with no session on purpose: it is the same answer for everyone, and
    // capturing it signed in would hide a handler that consulted one.
    let discover_map = call(
        &app,
        "GET",
        "/api/map?west=140.0&south=42.0&east=142.5&north=44.0&zoom=10",
        None,
        None,
    )
    .await;
    capture("discover_map", StatusCode::OK, &discover_map);
    let markers: Value = serde_json::from_str(&discover_map.body).expect("json");
    assert!(
        !markers["clusters"].as_array().expect("clusters").is_empty(),
        "the discovery map fixture has no markers in it, so the TypeScript check \
         would never see the shape of one: {}",
        discover_map.body
    );

    capture(
        "voyage_list",
        StatusCode::OK,
        &call(&app, "GET", "/api/voyages", None, None).await,
    );
    let mine = call(&app, "GET", "/api/voyages?mine=true", cookie, None).await;
    capture("voyage_list_mine", StatusCode::OK, &mine);
    let profile = call(&app, "GET", "/api/profiles/fixture-owner", None, None).await;
    capture("public_profile", StatusCode::OK, &profile);

    assert!(
        mine.body.contains(&draft_id.to_string()),
        "the owner's own listing should include the draft: {}",
        mine.body
    );

    // -- errors -------------------------------------------------------------
    //
    // The error body is part of the API surface: `code` is what a client
    // branches on. Four shapes, because they do not all come from the same
    // type — `AppError` produces the first two, and the two handler-local
    // `Rejection` types produce the others.

    capture(
        "error_not_found",
        StatusCode::NOT_FOUND,
        &call(
            &app,
            "GET",
            &format!("/api/voyages/{}", Uuid::now_v7()),
            cookie,
            None,
        )
        .await,
    );

    let unauthorized = call(&app, "GET", "/api/me", None, None).await;
    assert!(
        unauthorized.request_id.is_some(),
        "an error response above all must carry x-request-id"
    );
    capture(
        "error_unauthorized",
        StatusCode::UNAUTHORIZED,
        &unauthorized,
    );

    // Uploads require multipart content; JSON attachment requests are rejected.
    capture(
        "error_bad_request",
        StatusCode::BAD_REQUEST,
        &call(
            &app,
            "POST",
            &format!("/api/voyages/{voyage_id}/photos"),
            cookie,
            Some(json!({ "photo_ids": [placed] })),
        )
        .await,
    );

    // 422, which carries an extra `errors` array so the edit screen can put the
    // message beside the input rather than in a banner. Publishing a voyage
    // whose title is blank is the design's one publish precondition.
    let untitled = call(
        &app,
        "POST",
        "/api/voyages",
        cookie,
        Some(json!({ "title": "   " })),
    )
    .await;
    assert_eq!(untitled.status, StatusCode::CREATED, "{}", untitled.body);
    let untitled_id = serde_json::from_str::<Value>(&untitled.body).expect("json")["id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the create response names the voyage");

    capture(
        "error_validation",
        StatusCode::UNPROCESSABLE_ENTITY,
        &call(
            &app,
            "PATCH",
            &format!("/api/voyages/{untitled_id}"),
            cookie,
            Some(json!({ "published": true })),
        )
        .await,
    );

    // -- the endpoints with no body -----------------------------------------
    //
    // Nothing to capture, but they are part of the client's surface and a 204
    // that became a 200-with-a-body would be a silent contract change. Run last
    // and against the throwaway voyage, so the fixtures above describe the
    // world before any of this.

    for (label, uri) in [
        (
            "detach",
            format!("/api/voyages/{voyage_id}/photos/{unplaced}"),
        ),
        ("delete a draft", format!("/api/voyages/{draft_id}")),
        ("delete a voyage", format!("/api/voyages/{untitled_id}")),
    ] {
        let reply = call(&app, "DELETE", &uri, cookie, None).await;
        assert_eq!(
            reply.status,
            StatusCode::NO_CONTENT,
            "{label} ({uri}) answered {}: {}",
            reply.status,
            reply.body
        );
        assert!(reply.body.is_empty(), "{label} must carry no body");
    }

    let logout = call(&app, "POST", "/auth/logout", cookie, None).await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert!(logout.body.is_empty(), "a 204 must carry no body");

    // Confirm every expected fixture exists after comparison or regeneration.
    for name in [
        "me",
        "photo",
        "photo_list",
        "photo_upload",
        "voyage_created",
        "voyage_photos",
        "voyage_reordered",
        "voyage_updated",
        "voyage_detail",
        "voyage_map",
        "discover_map",
        "voyage_list",
        "voyage_list_mine",
        "public_profile",
        "error_not_found",
        "error_unauthorized",
        "error_bad_request",
        "error_validation",
    ] {
        let path = fixtures_dir().join(format!("{name}.json"));
        let written = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} was not written: {err}", path.display()));
        assert!(
            written.trim().len() > 2,
            "{} is empty, so the TypeScript contract test would be checking nothing",
            path.display()
        );
    }
}
