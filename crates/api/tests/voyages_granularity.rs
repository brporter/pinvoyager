//! Public granularity and bounded-query checks through the production router.
//!
//! The phase's central acceptance criterion is worded as an instruction:
//!
//! > With granularity set to ~10 km, grep every JSON response body across every
//! > endpoint for the exact stored coordinate; it appears nowhere.
//!
//! So that is what this file does, literally: it drives the **real router**
//! through `tower::ServiceExt::oneshot`, captures each response as text, and
//! searches the characters that actually went on the wire. Calling
//! `Granularity::apply` directly and asserting on the returned `f64` would test
//! the rounding function and nothing else — and the leak this criterion exists
//! to prevent is not a broken rounding function. It is a handler that forgot to
//! call it, a list endpoint nobody thought about, or an error message that
//! echoes the value back. Only a response body can show that.
//!
//! Two properties of the assertions are deliberate:
//!
//! * They compare **rendered digits**, not parsed floats. `43.061234` and
//!   `43.0612339999` are the same number to a comparison against an epsilon and
//!   are the same leak on the wire.
//! * Every leak sweep is paired with a check that the endpoint returned 200 and
//!   emitted *some* coordinate. An implementation that dropped coordinates
//!   entirely, or 404'd everywhere, would otherwise pass the grep perfectly.
//!
//! Writes go through the repository (`db::repo::voyages`) rather than through
//! the create/attach handlers, because the repository signatures are fixed by
//! contract while the request bodies are not. Reads go through HTTP, because
//! the serialized response *is* the thing under test.

#[path = "support/photo_voyage.rs"]
mod photo_voyage;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::config::Config;
use pinvoyager_api::db::repo::photos::{self, NewPhoto};
use pinvoyager_api::db::repo::voyages::{self, NewVoyage, VoyagePatch};
use pinvoyager_api::photos::worker::VariantQueue;
use pinvoyager_api::state::AppState;
use pinvoyager_api::voyages::Granularity;

// ---------------------------------------------------------------------------
// The coordinate under test.
//
// Distinctive on purpose. A round number like 43.0 / 141.0 would appear in a
// rounded response by coincidence — it is a plausible cell center — and the
// grep would then fail against a perfectly correct implementation. Six decimal
// places is also exactly what the column stores (`numeric(9, 6)`), so the value
// round-trips without the database itself rewriting the digits.
//
// Sapporo, roughly, which matters only in that it is nowhere near the equator:
// a longitude cell there is meaningfully wider in degrees than a latitude one,
// so an implementation that used a single fixed-degree grid for both axes has
// somewhere to be visibly wrong.
// ---------------------------------------------------------------------------

const EXACT_LAT: f64 = 43.061_234;
const EXACT_LON: f64 = 141.354_321;

/// The digits as `serde_json` renders them. Asserted as strings because that is
/// what a grep sees.
const EXACT_LAT_DIGITS: &str = "43.061234";
const EXACT_LON_DIGITS: &str = "141.354321";

/// What the sweep actually searches for: five decimal places, roughly one
/// meter.
///
/// A prefix rather than the full value, so that a leak dressed up as a
/// truncation — `43.06123`, or `43.061234000001` from a float round trip —
/// still trips it. It cannot collide with a legitimately rounded value: the
/// coarsest step this test uses is ~10 km, and a 10 km cell center agreeing
/// with the truth to one meter would be a coincidence of one in ten million,
/// not a rounding artifact.
const LAT_PREFIX: &str = "43.06123";
const LON_PREFIX: &str = "141.35432";

/// A second photo a few hundred meters from the first: ~220 m north and ~290 m
/// east, which is far enough apart to be distinct at exact precision and far
/// closer than the ~10 km cell they must collapse into.
const NEAR_LAT: f64 = 43.063_210;
const NEAR_LON: f64 = 141.357_890;

const DEFAULT_URL: &str = "postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("PINVOYAGER__DATABASE__URL"))
        .unwrap_or_else(|_| DEFAULT_URL.to_owned())
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

/// Built through the ordinary `Deserialize` path rather than by constructing
/// the struct, so a change to the configuration shape breaks this the same way
/// it breaks a deployment.
fn test_state(pool: PgPool) -> AppState {
    let config: Config = serde_json::from_value(json!({
        "server": {
            "bind": "127.0.0.1:0",
            "shutdown_timeout_secs": 5,
            // Generous: the N+1 test builds a 48-photo voyage, and a timeout
            // firing there would look like a routing failure.
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
            // The only way to get an authenticated session without credentials
            // for three providers. It is an authentication bypass and it is
            // compiled out of release builds; a debug test binary is the one
            // place it is meant to be used.
            "dev_login": true,
            "session": { "cookie_secure": false },
        },
        "photos": { "storage_root": std::env::temp_dir().join("pinvoyager-granularity-tests") },
    }))
    .expect("the test configuration should deserialize");

    AppState::new(
        pool,
        Arc::new(config),
        // No providers: nothing here signs in for real.
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    )
}

/// The production router, middleware and all — not a hand-assembled subset.
///
/// A subset would keep passing if `http::router` forgot to merge
/// `voyages::routes()`, which is the exact shape of "the one endpoint somebody
/// forgot" that this phase is about.
fn app(pool: PgPool) -> Router {
    pinvoyager_api::http::router(test_state(pool))
}

/// One captured response: the status, and every character the client could see.
struct Reply {
    status: StatusCode,
    /// Status line, headers and body concatenated. Headers are included because
    /// "it appears nowhere" is a claim about the response, and a coordinate in a
    /// `Location` or a diagnostic header has left the server just as surely as
    /// one in the body.
    transcript: String,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("response body is not JSON ({err}): {}", self.body))
    }
}

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> Reply {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    send(app, request.body(Body::empty()).expect("valid request")).await
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router is infallible");

    let status = response.status();
    let mut transcript = format!("HTTP {status}\n");
    for (name, value) in response.headers() {
        transcript.push_str(&format!(
            "{name}: {}\n",
            value.to_str().unwrap_or("<binary>")
        ));
    }

    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("response body should be readable");
    let body = String::from_utf8_lossy(&bytes).into_owned();

    transcript.push('\n');
    transcript.push_str(&body);

    Reply {
        status,
        transcript,
        body,
    }
}

/// An authenticated caller.
struct Caller {
    user_id: Uuid,
    /// A `Cookie:` header value, ready to send back.
    cookie: String,
}

/// Signs in through the dev login, which forges a session without a provider.
async fn sign_in(app: &Router, subject: &str) -> Caller {
    // Unique per call: two tests running against the same database would
    // otherwise share an account, and "a stranger" would be the owner.
    let subject = format!("{subject}-{}", Uuid::now_v7());

    let request = Request::builder()
        .method("POST")
        .uri("/auth/dev/login")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "subject": subject, "display_name": "Test" }).to_string(),
        ))
        .expect("valid request");

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router is infallible");

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the dev login must succeed, or no test in this file has a session"
    );

    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .expect("the dev login should set a session cookie")
        // Just the name=value pair; the attributes are for a browser.
        .split(';')
        .next()
        .expect("a cookie header has at least one segment")
        .to_owned();

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body is small");
    let body: Value = serde_json::from_slice(&bytes).expect("the dev login answers with JSON");
    let user_id = body["user_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the dev login reports the user id it signed in as");

    Caller { user_id, cookie }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn photo_at(pool: &PgPool, owner: Uuid, at: Option<(f64, f64)>) -> Uuid {
    let photo = photos::create(
        pool,
        &NewPhoto {
            voyage_id: photo_voyage::fixture_voyage(pool, owner).await,
            owner_id: owner,
            // Content-addressed and unique; nothing here reads the bytes.
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            byte_size: 4096,
            width: 4032,
            height: 3024,
            original_format: "jpeg".to_owned(),
            captured_at: None,
            latitude: at.map(|(lat, _)| lat),
            longitude: at.map(|(_, lon)| lon),
        },
    )
    .await
    .expect("create photo");

    photo.id
}

async fn voyage_with(
    pool: &PgPool,
    owner: Uuid,
    granularity: Granularity,
    published: bool,
    photo_ids: &[Uuid],
) -> Uuid {
    let voyage = voyages::create(
        pool,
        &NewVoyage {
            owner_id: owner,
            title: "Hokkaido".to_owned(),
            description: Some("A voyage".to_owned()),
            granularity: granularity.as_str().to_owned(),
        },
    )
    .await
    .expect("create voyage");

    if !photo_ids.is_empty() {
        let attached = voyages::move_photos(pool, voyage.id, owner, photo_ids)
            .await
            .expect("attach photos");
        assert_eq!(
            attached as usize,
            photo_ids.len(),
            "every photo belongs to the owner, so every one should have attached"
        );
    }

    if published {
        set_granularity_and_publish(pool, voyage.id, owner, None, Some(true)).await;
    }

    voyage.id
}

async fn set_granularity_and_publish(
    pool: &PgPool,
    voyage: Uuid,
    owner: Uuid,
    granularity: Option<Granularity>,
    published: Option<bool>,
) {
    voyages::update(
        pool,
        voyage,
        owner,
        &VoyagePatch {
            granularity: granularity.map(|g| g.as_str().to_owned()),
            published,
            ..Default::default()
        },
    )
    .await
    .expect("update voyage")
    .expect("the owner's own voyage should be updatable");
}

// ---------------------------------------------------------------------------
// Reading coordinates back out of a response
//
// The response shapes belong to the handlers, which are being written in
// parallel with this file, so nothing here assumes a particular nesting. These
// walk the whole document looking for coordinate-shaped objects, which also
// means a coordinate hiding somewhere unexpected — an embedded summary, a
// "cover photo" — is found rather than missed.
// ---------------------------------------------------------------------------

/// The two coordinate values exactly as rendered, e.g. `"43.0741,141.3873"`.
///
/// Rendered rather than parsed: byte-identical output is the property the
/// stability test is about, and `f64` comparison would silently accept two
/// different serializations of nearly the same number.
fn rendered_coordinate(object: &Map<String, Value>) -> Option<String> {
    const SPELLINGS: [(&str, &str); 3] =
        [("latitude", "longitude"), ("lat", "lon"), ("lat", "lng")];

    SPELLINGS.iter().find_map(|(lat_key, lon_key)| {
        let lat = object.get(*lat_key)?;
        let lon = object.get(*lon_key)?;
        if lat.is_null() || lon.is_null() {
            return None;
        }
        Some(format!("{lat},{lon}"))
    })
}

/// The coordinate an object carries, whether flattened into it or nested in a
/// child (`"coordinate": { ... }`, `"location": { ... }`).
fn coordinate_of(object: &Map<String, Value>) -> Option<String> {
    rendered_coordinate(object).or_else(|| {
        object
            .values()
            .filter_map(Value::as_object)
            .find_map(rendered_coordinate)
    })
}

fn photo_id_of(object: &Map<String, Value>) -> Option<String> {
    object
        .get("photo_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Every coordinate in a response, keyed by the id of the object carrying it.
fn coordinates_by_id(value: &Value) -> BTreeMap<String, String> {
    fn walk(value: &Value, out: &mut BTreeMap<String, String>) {
        match value {
            Value::Object(object) => {
                if let (Some(id), Some(coordinate)) = (photo_id_of(object), coordinate_of(object)) {
                    out.insert(id, coordinate);
                }
                for child in object.values() {
                    walk(child, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }

    let mut out = BTreeMap::new();
    walk(value, &mut out);
    out
}

/// The object describing a given photo, wherever it sits in the document.
fn object_for(value: &Value, id: Uuid) -> Option<&Map<String, Value>> {
    let wanted = id.to_string();
    match value {
        Value::Object(object) => {
            if photo_id_of(object).as_deref() == Some(wanted.as_str()) {
                return Some(object);
            }
            object.values().find_map(|child| object_for(child, id))
        }
        Value::Array(items) => items.iter().find_map(|item| object_for(item, id)),
        _ => None,
    }
}

/// Whether an object says, in one of the plausible spellings, that this photo
/// has no location.
///
/// Deliberately lenient about the spelling and strict about the fact: the
/// requirement is that a reader can *tell*, not that a particular key was
/// chosen. Photo responses use `placed` to distinguish missing GPS.
fn flagged_unplaced(object: &Map<String, Value>) -> bool {
    object.get("placed") == Some(&Value::Bool(false))
        || object.get("unplaced") == Some(&Value::Bool(true))
        || object.get("coordinate") == Some(&Value::Null)
        || object.get("location") == Some(&Value::Null)
        || object.get("latitude") == Some(&Value::Null)
}

// ---------------------------------------------------------------------------
// The grep
// ---------------------------------------------------------------------------

#[track_caller]
fn assert_no_exact_coordinate(label: &str, reply: &Reply) {
    for needle in [LAT_PREFIX, LON_PREFIX] {
        assert!(
            !reply.transcript.contains(needle),
            "T5 FAILED — the exact stored coordinate leaked from {label}.\n\
             Found {needle:?} in a response for a voyage set to ~10 km granularity.\n\
             Full response:\n{}",
            reply.transcript
        );
    }
}

// ---------------------------------------------------------------------------
// A self-check on the harness.
//
// Every assertion below is "the coordinate is absent", and a route that does
// not exist satisfies all of them perfectly. If `http::router` does not merge
// `voyages::routes()`, this is the one test that says so — instead of a file
// full of green that proves nothing.
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn the_voyage_routes_are_reachable_through_the_real_router(pool: PgPool) {
    let app = app(pool.clone());
    let owner = sign_in(&app, "reachability").await;
    let photo = photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await;
    let voyage = voyage_with(&pool, owner.user_id, Granularity::Km10, true, &[photo]).await;

    for uri in [
        "/api/voyages".to_owned(),
        format!("/api/voyages/{voyage}"),
        format!("/api/voyages/{voyage}/map"),
    ] {
        let reply = get(&app, &uri, None).await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{uri} did not answer 200 to an anonymous caller. If this is a 404, the voyage \
             routes are not merged into `http::router` and every leak assertion in this file \
             is vacuous.\n{}",
            reply.transcript
        );
    }
}

// ---------------------------------------------------------------------------
// T5 — the leak sweep. The phase's central check.
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn the_exact_coordinate_appears_in_no_response_at_km10(pool: PgPool) {
    let app = app(pool.clone());
    let owner = sign_in(&app, "leak-owner").await;
    let stranger = sign_in(&app, "leak-stranger").await;

    let photo = photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await;
    let voyage = voyage_with(&pool, owner.user_id, Granularity::Km10, true, &[photo]).await;

    // Three callers, because granularity is a property of the voyage and not of
    // the reader. The owner is included on purpose: the plan says *every*
    // coordinate the API emits for a voyage is transformed to that voyage's
    // precision, and a per-caller exemption would be a second code path through
    // the boundary — which is the arrangement the whole module exists to avoid.
    let callers: [(&str, Option<&str>); 3] = [
        ("an anonymous caller", None),
        ("a stranger", Some(stranger.cookie.as_str())),
        ("the owner", Some(owner.cookie.as_str())),
    ];

    for (who, cookie) in callers {
        let mut sweep = vec![
            // The detail view: the obvious one.
            format!("/api/voyages/{voyage}"),
            // The discovery list: the endpoint nobody thinks to check, and the
            // one that leaks at full precision if a summary carries a center.
            "/api/voyages".to_owned(),
            "/api/voyages?limit=50".to_owned(),
            // The map payload, which exists precisely to carry coordinates.
            format!("/api/voyages/{voyage}/map"),
            // An error path. "no photo at 43.06, 141.35" is a leak with a 404
            // attached to it.
            format!("/api/voyages/{}", Uuid::now_v7()),
        ];

        // Exact photo metadata remains owner-only, including for published
        // voyages. Public callers must use the rounded voyage responses.
        if who != "the owner" {
            sweep.push(format!("/api/photos/{photo}"));
        }

        for uri in sweep {
            let reply = get(&app, &uri, cookie).await;
            assert_no_exact_coordinate(&format!("GET {uri} as {who}"), &reply);
        }
    }

    // ---- The one exemption, written out rather than left implicit ----
    //
    // `GET /api/photos/{id}` returns the exact coordinate **to the owner of the
    // photograph**, and that is correct: it is their own library, holding the
    // value their own camera recorded, and granularity is a promise about what
    // the *public* sees (D6), not a promise to hide a user's data from
    // themselves. Rounding here would also mean a second implementation of the
    // rule in a second module, which is how two roundings come to disagree.
    //
    // Asserted rather than merely commented, on the same principle as
    // `integration.rs`'s PERMITTED list: an exemption nobody re-checks is an
    // exemption that outlives its reason.
    let own_library = get(&app, &format!("/api/photos/{photo}"), Some(&owner.cookie)).await;
    assert_eq!(own_library.status, StatusCode::OK);
    assert!(
        own_library.body.contains(EXACT_LAT_DIGITS),
        "the owner's own library no longer returns the exact coordinate. That may well be an \
         improvement — but if it is deliberate, delete this exemption and put \
         `/api/photos/{{id}}` back into the owner's sweep above, so the rule is enforced rather \
         than assumed.\n{}",
        own_library.body
    );

    // The other half, and the reason the grep alone is not enough: an
    // implementation that emitted no coordinates at all — or that 404'd
    // everywhere — would pass every assertion above.
    let detail = get(&app, &format!("/api/voyages/{voyage}"), None).await;
    assert_eq!(detail.status, StatusCode::OK);
    let coarse = coordinates_by_id(&detail.json());
    let coarse = coarse.get(&photo.to_string()).unwrap_or_else(|| {
        panic!(
            "the voyage detail carried no coordinate for its only photo, so the sweep above \
             proved nothing:\n{}",
            detail.body
        )
    });

    let map = get(&app, &format!("/api/voyages/{voyage}/map"), None).await;
    assert_eq!(map.status, StatusCode::OK);
    assert!(
        !coordinates_by_id(&map.json()).is_empty(),
        "the map endpoint returned no coordinates at all, so its leak sweep proved nothing:\n{}",
        map.body
    );

    // Loosening granularity must return the full value. Without this the whole
    // feature could be implemented as "discard the coordinates", which passes
    // T5 and makes the setting one-way and useless — the interaction with phase
    // 4 that the plan calls out explicitly.
    set_granularity_and_publish(&pool, voyage, owner.user_id, Some(Granularity::Exact), None).await;

    let exact = get(&app, &format!("/api/voyages/{voyage}"), None).await;
    assert_eq!(exact.status, StatusCode::OK);
    assert!(
        exact.body.contains(EXACT_LAT_DIGITS) && exact.body.contains(EXACT_LON_DIGITS),
        "at `exact` the stored coordinate {EXACT_LAT_DIGITS},{EXACT_LON_DIGITS} must be returned \
         in full. Getting here means granularity is not reversible — the exact value is gone, \
         not hidden.\n{}",
        exact.body
    );

    let fine = coordinates_by_id(&exact.json());
    let fine = fine
        .get(&photo.to_string())
        .expect("the photo's coordinate at exact precision");
    assert_ne!(
        coarse, fine,
        "the ~10 km reading and the exact reading are identical, which means the km10 response \
         was never rounded at all"
    );
}

// ---------------------------------------------------------------------------
// T5 — stability.
// ---------------------------------------------------------------------------

/// Two reads of the same voyage must return byte-identical coordinates.
///
/// If the transform were a random offset within a radius rather than a fixed
/// grid, this fails — and both consequences are bad. Pins would visibly jitter
/// between page loads, and, far worse, an observer who reloads the page a few
/// hundred times can average the samples and recover the true position to a
/// precision the user never agreed to. Determinism is what makes the radius a
/// guarantee instead of a suggestion.
#[sqlx::test]
async fn rounding_is_stable_across_reads(pool: PgPool) {
    let app = app(pool.clone());
    let owner = sign_in(&app, "stability").await;

    let photo = photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await;
    let voyage = voyage_with(&pool, owner.user_id, Granularity::Km10, true, &[photo]).await;
    let uri = format!("/api/voyages/{voyage}");

    let first = get(&app, &uri, None).await;
    let second = get(&app, &uri, None).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(second.status, StatusCode::OK);

    let first = coordinates_by_id(&first.json());
    let second = coordinates_by_id(&second.json());

    assert!(
        !first.is_empty(),
        "no coordinates were returned, so stability was not tested"
    );
    assert_eq!(
        first, second,
        "the same voyage returned different coordinates on two successive reads; repeated \
         sampling would average this noise away and recover the exact position"
    );
}

// ---------------------------------------------------------------------------
// T5 — clustering.
// ---------------------------------------------------------------------------

/// Two photos a few hundred meters apart report the *same* coordinate at ~10 km.
///
/// This is the specified behavior — "granularity semantics just lump photos
/// together in a middle-cluster for the point" — and it is also what makes the
/// setting convincing. A coarse voyage that drew twelve distinct pins in a
/// tight, evenly-spaced scatter would advertise that each one had been nudged
/// independently, and the shape of the scatter would say a great deal about the
/// shape of the truth.
#[sqlx::test]
async fn nearby_photos_collapse_to_one_point_at_km10(pool: PgPool) {
    let app = app(pool.clone());
    let owner = sign_in(&app, "clustering").await;

    let first = photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await;
    let second = photo_at(&pool, owner.user_id, Some((NEAR_LAT, NEAR_LON))).await;
    let voyage = voyage_with(
        &pool,
        owner.user_id,
        Granularity::Km10,
        true,
        &[first, second],
    )
    .await;

    let detail = get(&app, &format!("/api/voyages/{voyage}"), None).await;
    assert_eq!(detail.status, StatusCode::OK);
    let coordinates = coordinates_by_id(&detail.json());

    let a = coordinates
        .get(&first.to_string())
        .unwrap_or_else(|| panic!("no coordinate for the first photo:\n{}", detail.body));
    let b = coordinates
        .get(&second.to_string())
        .unwrap_or_else(|| panic!("no coordinate for the second photo:\n{}", detail.body));

    assert_eq!(
        a, b,
        "two photos ~360 m apart reported different points in a ~10 km voyage. Either the \
         rounding is per-photo noise rather than a shared grid, or these two points straddle a \
         cell boundary — the offset is 3-4% of a cell and both sit mid-cell for a grid anchored \
         at 0°, so a boundary here would be worth checking before changing the test."
    );
}

// ---------------------------------------------------------------------------
// T6 — draft visibility.
// ---------------------------------------------------------------------------

/// A draft is 404 to everyone but its owner — and specifically **404, not 403**.
///
/// The status code is the whole test. A 403 is an honest answer to "may I read
/// this?" and a disastrous answer to "does this exist?": it turns id probing
/// into an enumeration oracle for unpublished work, which for a travel diary
/// means confirming that somebody has an unpublished trip before they have
/// chosen to say so. Asserting merely "not 200" would accept exactly that.
#[sqlx::test]
async fn a_draft_is_404_to_everyone_but_its_owner(pool: PgPool) {
    let app = app(pool.clone());
    let owner = sign_in(&app, "draft-owner").await;
    let stranger = sign_in(&app, "draft-stranger").await;

    let photo = photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await;
    let draft = voyage_with(&pool, owner.user_id, Granularity::Km10, false, &[photo]).await;

    for uri in [
        format!("/api/voyages/{draft}"),
        format!("/api/voyages/{draft}/map"),
    ] {
        for (who, cookie) in [
            ("an anonymous caller", None),
            ("a stranger", Some(stranger.cookie.as_str())),
        ] {
            let reply = get(&app, &uri, cookie).await;
            assert_eq!(
                reply.status,
                StatusCode::NOT_FOUND,
                "{uri} answered {} to {who}. A draft must be 404: 403 confirms that a voyage \
                 exists at this id, which is the leak this rule exists to prevent.\n{}",
                reply.status,
                reply.transcript
            );
        }

        let owned = get(&app, &uri, Some(owner.cookie.as_str())).await;
        assert_eq!(
            owned.status,
            StatusCode::OK,
            "{uri} must be readable by its owner, or the 404 above is just a broken route.\n{}",
            owned.transcript
        );
    }

    // The other way a draft leaks: by appearing in the public feed.
    let feed = get(&app, "/api/voyages", None).await;
    assert_eq!(feed.status, StatusCode::OK);
    assert!(
        !feed.body.contains(&draft.to_string()),
        "an unpublished voyage appeared in the discovery feed:\n{}",
        feed.body
    );
}

// ---------------------------------------------------------------------------
// T6/T9 — no N+1.
//
// Counted at the PostgreSQL wire protocol, by putting a TCP proxy between the
// pool and the server and counting frontend `Query` and `Execute` messages.
//
// Why not something simpler:
//
// * `pg_stat_statements` counts *calls per normalized statement* and is an
//   extension that may not be installed. It also aggregates across the whole
//   server, so a concurrently running test pollutes it.
// * Timing is not a count. A test that asserts "loading 48 photos takes less
//   than n times as long as loading 4" is measuring the machine it runs on, and
//   it passes on a fast laptop against an implementation that issues 48 queries.
// * sqlx's own query logging goes through `tracing`/`log` at a level and target
//   that a test would have to guess at, and it would be silently disabled by a
//   subscriber another test installed first.
//
// The proxy counts what actually crossed the socket, which is the thing the
// criterion is about, and it cannot be fooled by a loop that *looks*
// concurrent.
// ---------------------------------------------------------------------------

/// Counts statement executions on every connection routed through it.
struct StatementCounter {
    addr: SocketAddr,
    count: Arc<AtomicUsize>,
    accept: tokio::task::JoinHandle<()>,
}

impl Drop for StatementCounter {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl StatementCounter {
    async fn in_front_of(upstream: SocketAddr) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral loopback port should succeed");
        let addr = listener
            .local_addr()
            .expect("the listener should report its address");
        let count = Arc::new(AtomicUsize::new(0));

        let accept = tokio::spawn({
            let count = Arc::clone(&count);
            async move {
                while let Ok((client, _)) = listener.accept().await {
                    let Ok(server) = tokio::net::TcpStream::connect(upstream).await else {
                        continue;
                    };
                    tokio::spawn(relay(client, server, Arc::clone(&count)));
                }
            }
        });

        Self {
            addr,
            count,
            accept,
        }
    }

    fn reset(&self) {
        self.count.store(0, Ordering::SeqCst);
    }

    fn statements(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

/// Forwards a connection in both directions, parsing only the client's half.
async fn relay(
    client: tokio::net::TcpStream,
    server: tokio::net::TcpStream,
    count: Arc<AtomicUsize>,
) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let (mut from_client, mut to_client) = client.into_split();
    let (mut from_server, mut to_server) = server.into_split();

    let upstream = async move {
        let mut parser = FrontendParser::default();
        let mut buffer = vec![0u8; 32 * 1024];
        loop {
            match from_client.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // Forwarded before it is parsed: the proxy must never be the
                    // reason a query is slow, or the thing it measures changes.
                    if to_server.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                    parser.consume(&buffer[..n], &count);
                }
            }
        }
        let _ = to_server.shutdown().await;
    };

    let downstream = async move {
        let _ = tokio::io::copy(&mut from_server, &mut to_client).await;
    };

    tokio::join!(upstream, downstream);
}

/// A frame-accurate reader of the PostgreSQL v3 frontend protocol.
///
/// Only the framing is parsed, never the payload — enough to know where each
/// message starts and what type byte it carries. Counting message boundaries
/// rather than searching the byte stream for SQL keywords is what keeps this
/// exact: a query whose *text* contains the word "select" is one statement.
#[derive(Default)]
struct FrontendParser {
    buffer: Vec<u8>,
    /// The startup packet has no type byte, so the first message must be
    /// handled differently from every one that follows.
    startup_done: bool,
}

impl FrontendParser {
    fn consume(&mut self, bytes: &[u8], count: &AtomicUsize) {
        self.buffer.extend_from_slice(bytes);

        let mut at = 0usize;
        loop {
            let remaining = &self.buffer[at..];

            if !self.startup_done {
                if remaining.len() < 4 {
                    break;
                }
                let length =
                    u32::from_be_bytes([remaining[0], remaining[1], remaining[2], remaining[3]])
                        as usize;
                if length < 8 || remaining.len() < length {
                    break;
                }
                let code =
                    u32::from_be_bytes([remaining[4], remaining[5], remaining[6], remaining[7]]);
                // An SSLRequest or GSSENCRequest is a preamble, not the startup
                // packet — the real one follows it. (The connection string used
                // here disables TLS, so this should not arise; if it ever did
                // and TLS were negotiated, everything after would be ciphertext
                // and the count would be nonsense rather than merely wrong,
                // which is why it is worth being explicit about.)
                let preamble = length == 8 && (code == 80_877_103 || code == 80_877_104);
                if !preamble {
                    self.startup_done = true;
                }
                at += length;
            } else {
                if remaining.len() < 5 {
                    break;
                }
                let length =
                    u32::from_be_bytes([remaining[1], remaining[2], remaining[3], remaining[4]])
                        as usize;
                if length < 4 || remaining.len() < length + 1 {
                    break;
                }
                // `Execute` runs a bound portal; `Query` is the simple-protocol
                // equivalent. Parse, Bind, Describe and Sync are excluded — they
                // are per-statement bookkeeping, and counting them would make a
                // cold prepared-statement cache look like an N+1.
                if remaining[0] == b'E' || remaining[0] == b'Q' {
                    count.fetch_add(1, Ordering::SeqCst);
                }
                at += length + 1;
            }
        }

        self.buffer.drain(..at);
    }
}

/// Where the test database actually lives, and under what name.
async fn upstream_and_database(pool: &PgPool) -> (SocketAddr, String, String, String) {
    let url = url::Url::parse(&database_url()).expect("DATABASE_URL should be a URL");
    let host = url.host_str().unwrap_or("127.0.0.1").to_owned();
    let port = url.port().unwrap_or(5432);
    let user = url.username().to_owned();
    let password = url.password().unwrap_or_default().to_owned();

    // `#[sqlx::test]` hands out a pool onto a database it created for this test
    // alone; the name is generated, so ask it rather than guessing.
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await
        .expect("the test database should name itself");

    let upstream = tokio::net::lookup_host((host.as_str(), port))
        .await
        .expect("the database host should resolve")
        .next()
        .expect("the database host should resolve to at least one address");

    (upstream, database, user, password)
}

#[sqlx::test]
async fn loading_a_voyage_issues_a_bounded_number_of_queries(pool: PgPool) {
    let (upstream, database, user, password) = upstream_and_database(&pool).await;
    let counter = StatementCounter::in_front_of(upstream).await;

    // `sslmode=disable` so the wire stays parseable: a negotiated TLS session
    // would turn every frontend message into ciphertext and the counter into a
    // random number generator.
    let proxied = sqlx::postgres::PgPoolOptions::new()
        // Two, not one: a handler that opens a transaction and also queries the
        // pool would deadlock on a single connection, and a deadlock is a much
        // worse failure than a wrong number.
        .max_connections(2)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&format!(
            "postgres://{user}:{password}@{}:{}/{database}?sslmode=disable",
            counter.addr.ip(),
            counter.addr.port(),
        ))
        .await
        .expect("connecting through the counting proxy should succeed");

    // Open every connection the pool will ever hold, before anything is
    // counted: establishing one costs a handshake and a few setup statements,
    // and one arriving mid-measurement would be indistinguishable from an N+1.
    {
        let held = (
            proxied.acquire().await.expect("first connection"),
            proxied.acquire().await.expect("second connection"),
        );
        drop(held);
    }

    // Calibration. This is what makes the assertion at the end of the test mean
    // something: 48 statements are issued deliberately, and the counter has to
    // see 48. Without it, a counter that had silently stopped parsing — a
    // protocol change, a TLS upgrade, a framing bug — would report a
    // beautifully small number for every implementation, N+1 or not.
    counter.reset();
    for _ in 0..48 {
        let _: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&proxied)
            .await
            .expect("a trivial query should succeed through the proxy");
    }
    assert_eq!(
        counter.statements(),
        48,
        "the wire counter mis-counted 48 deliberate statements, so nothing it reports below can \
         be trusted"
    );

    let app = app(proxied.clone());
    let owner = sign_in(&app, "n-plus-one").await;

    // Two voyages differing only in photo count. Comparing them is what makes
    // the assertion independent of how many queries the *rest* of the request
    // costs — the session lookup, the voyage row, whatever a future middleware
    // adds. Only the growth matters.
    let mut small = Vec::new();
    for _ in 0..4 {
        small.push(photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await);
    }
    let mut large = Vec::new();
    for index in 0..48 {
        // Spread out so no two share a cell; a clustering shortcut must not be
        // what keeps the query count down.
        let lat = EXACT_LAT + f64::from(index) * 0.01;
        large.push(photo_at(&pool, owner.user_id, Some((lat, EXACT_LON))).await);
    }

    let small = voyage_with(&pool, owner.user_id, Granularity::Km10, true, &small).await;
    let large = voyage_with(&pool, owner.user_id, Granularity::Km10, true, &large).await;

    // Warm-up: connection setup, the pool's own round trips and the first use of
    // each statement are one-off costs, and none of them are what the criterion
    // is about.
    for uri in [
        format!("/api/voyages/{small}"),
        format!("/api/voyages/{large}"),
    ] {
        let reply = get(&app, &uri, None).await;
        assert_eq!(reply.status, StatusCode::OK, "{uri}\n{}", reply.transcript);
    }

    counter.reset();
    let reply = get(&app, &format!("/api/voyages/{small}"), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let four_photos = counter.statements();

    counter.reset();
    let reply = get(&app, &format!("/api/voyages/{large}"), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let forty_eight_photos = counter.statements();

    // If the counter sees nothing at all it is broken, and every comparison
    // below would be 0 <= 0. Reading a voyage cannot cost fewer than two
    // statements — the voyage row and its photos.
    assert!(
        four_photos >= 2,
        "the wire counter saw {four_photos} statements for a voyage read, which means it is not \
         counting; the comparison below would pass regardless of the implementation"
    );

    assert!(
        forty_eight_photos <= four_photos + 2,
        "loading 48 photos cost {forty_eight_photos} statements against {four_photos} for 4. The \
         cost is scaling with photo count, which is an `await` inside a loop on the product's \
         most-visited endpoint — it looks concurrent, it serializes every round trip, and it gets \
         worse with every photo a user adds."
    );

    // Close before the test ends: `#[sqlx::test]` drops the database it made,
    // and an open connection from a second pool is what makes that fail.
    proxied.close().await;
}

// ---------------------------------------------------------------------------
// T6 — unplaced photos.
// ---------------------------------------------------------------------------

/// A photo without GPS is present and flagged, not silently dropped.
///
/// Absent coordinates are ordinary — screenshots, scans, anything shot with
/// location services off — so a voyage full of them is a normal voyage, not an
/// empty one. Two failure modes are being ruled out here, and dropping the
/// photo is the milder: a user who attached twelve photographs and sees nine
/// concludes the upload lost three. The other is inventing a position for it,
/// which is why the assertion also requires that no coordinate came back.
#[sqlx::test]
async fn unplaced_photos_are_present_and_flagged(pool: PgPool) {
    let app = app(pool.clone());
    let owner = sign_in(&app, "unplaced").await;

    let placed = photo_at(&pool, owner.user_id, Some((EXACT_LAT, EXACT_LON))).await;
    let unplaced = photo_at(&pool, owner.user_id, None).await;
    let voyage = voyage_with(
        &pool,
        owner.user_id,
        Granularity::Km10,
        true,
        &[placed, unplaced],
    )
    .await;

    // The detail view is the voyage as its author sees it, so the photograph
    // itself has to be in there, flagged.
    let uri = format!("/api/voyages/{voyage}");
    let reply = get(&app, &uri, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.transcript);
    let body = reply.json();

    let object = object_for(&body, unplaced).unwrap_or_else(|| {
        panic!(
            "the photo with no coordinates is missing from {uri}. A photo that vanishes from a \
             response is indistinguishable, to its owner, from a photo that was lost.\n{}",
            reply.body
        )
    });

    assert!(
        coordinate_of(object).is_none(),
        "a coordinate was reported for a photo that has none, in {uri}. Inventing a position is \
         worse than omitting one: it is indistinguishable from a real reading.\n{}",
        reply.body
    );
    assert!(
        flagged_unplaced(object),
        "the unplaced photo in {uri} carries no flag saying so, so a client cannot tell it apart \
         from one whose coordinate simply failed to load:\n{}",
        reply.body
    );

    // The placed one is still placed: a response that dropped every coordinate
    // would satisfy both assertions above.
    assert!(
        coordinates_by_id(&body).contains_key(&placed.to_string()),
        "the placed photo lost its coordinate in {uri}:\n{}",
        reply.body
    );

    // The map payload is allowed to be leaner — it exists to draw pins, and a
    // photograph with no position is not a pin. What it may **not** do is stay
    // silent about it: a caller that renders only this response must be able to
    // tell that two photographs were attached and one is missing from the map,
    // or the UI shows a voyage that has quietly shed a photo. Either spelling
    // satisfies that; being unaccounted for does not.
    let uri = format!("/api/voyages/{voyage}/map");
    let reply = get(&app, &uri, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.transcript);
    let body = reply.json();

    let listed = object_for(&body, unplaced)
        .is_some_and(|object| coordinate_of(object).is_none() && flagged_unplaced(object));
    let counted = ["unplaced_count", "unplaced", "without_location"]
        .iter()
        .any(|key| body.get(*key).and_then(Value::as_u64) == Some(1));

    assert!(
        listed || counted,
        "{uri} neither lists the unplaced photo nor reports how many photos have no location. A \
         map payload may reasonably carry only pins, but a photograph that is in the voyage and \
         absent from the response without a word is one the client will report as lost.\n{}",
        reply.body
    );
    assert!(
        coordinates_by_id(&body).contains_key(&placed.to_string()),
        "the placed photo has no pin in {uri}:\n{}",
        reply.body
    );
}
