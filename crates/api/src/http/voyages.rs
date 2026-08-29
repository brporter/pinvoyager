//! Voyage endpoints.
//!
//! CONTRACT — `routes()` is fixed; handlers are owned by the CRUD and read
//! agents.
//!
//! ```text
//! POST   /api/voyages                       create (draft)
//! GET    /api/voyages                       discovery feed, public
//! GET    /api/voyages/{id}                  read; public if published
//! PATCH  /api/voyages/{id}                  owner only
//! DELETE /api/voyages/{id}                  owner only
//! POST   /api/voyages/{id}/photos           attach
//! DELETE /api/voyages/{id}/photos/{photo}   detach
//! PATCH  /api/voyages/{id}/photos/order     reorder
//! GET    /api/voyages/{id}/map              lean payload for phase 7
//! ```
//!
//! # This module is the granularity boundary
//!
//! Every coordinate the service emits passes through here, and the only type
//! that can carry one into a response is
//! [`crate::voyages::Coordinate`], which cannot be constructed without a
//! [`crate::voyages::Granularity`]. The repository's row types hold exact
//! values and do not implement `Serialize`, so the compiler refuses the
//! shortcut.
//!
//! Two things follow that are easy to get wrong:
//!
//! * **The list endpoint needs it too.** A voyage summary that carried a
//!   "center" coordinate would leak at exactly the same precision as the
//!   detail view, and it is the endpoint nobody thinks to check.
//! * **So do error messages.** An error that echoes a coordinate back — "no
//!   photo at 43.06, 141.35" — leaks it just as effectively as a success.
//!
//! # Draft visibility
//!
//! A draft returns **404** to anyone but its owner, never 403. Returning
//! "forbidden" confirms that a voyage exists at that id, which turns id
//! probing into an enumeration oracle for unpublished work.
//!
//! # How the one rule is made structural here
//!
//! Three properties, each of which is a compile-time fact rather than a
//! convention someone has to remember:
//!
//! 1. **No response type in this file has an `f64` field.** The only way a
//!    latitude reaches a body is the `coordinate` field of
//!    [`VoyagePhotoResponse`] or [`MapPin`], both typed
//!    [`crate::voyages::Coordinate`]. Writing `latitude: photo.latitude` into
//!    a serializable struct does not compile, because the row type is not
//!    `Serialize` and `Coordinate` has no constructor from a number.
//! 2. **There is exactly one call to `Granularity::apply` in the module**, in
//!    [`place`]. Every endpoint — detail, feed, map — reaches a coordinate
//!    through it. A new endpoint added at six o'clock on a Friday either calls
//!    it or has no coordinate to emit.
//! 3. **Every error message in this module is a `&'static str`.** The field
//!    error type carries `&'static str`, not `String`, so there is no
//!    `format!` for a coordinate to be interpolated into. The one variant that
//!    can carry runtime text — `AppError::BadRequest` — is only ever handed a
//!    constant from the list at the top of this file.
//!
//! An unknown stored granularity fails **closed**, at the coarsest setting:
//! see [`stored_granularity`]. A build that meets a spelling a later migration
//! added must not decide that the safe reading is `exact`.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::{CurrentUser, MaybeUser};
use crate::db::repo::photos as photos_repo;
use crate::db::repo::voyages::{self as repo, NewVoyage, Voyage, VoyagePatch, VoyagePhoto};
use crate::error::AppError;
use crate::photos::VariantKind;
use crate::state::AppState;
use crate::voyages::{Coordinate, Granularity};

/// Default and maximum page size for the listing endpoints.
const DEFAULT_PAGE: i64 = 20;
const MAX_PAGE: i64 = 50;

/// Bounds on the two free-text columns. Nothing legitimate is close to either;
/// they exist so a caller cannot use `voyages` as free storage, and so the edit
/// screen has a limit to show rather than discovering one from a 500.
const MAX_TITLE_LEN: usize = 200;
const MAX_DESCRIPTION_LEN: usize = 8_000;

/// Bound on a single attach or reorder request. A voyage of fifty photographs
/// is the design's example; five hundred in one request is already generous and
/// keeps the statement the repository issues a bounded size.
const MAX_PHOTO_IDS: usize = 500;

// Every client-visible message in this module, as constants, because the type
// that carries them is `&'static str`. That is the mechanical half of "an error
// must not echo a coordinate": there is no `format!` here to interpolate one
// into.
const TITLE_REQUIRED: &str = "A title is required before publishing.";
const TITLE_TOO_LONG: &str = "That title is longer than we can store.";
const DESCRIPTION_TOO_LONG: &str = "That description is longer than we can store.";
const UNKNOWN_GRANULARITY: &str = "That is not a location precision we recognize.";
const NO_PHOTOS_NAMED: &str = "Name at least one photograph.";
const TOO_MANY_PHOTOS: &str = "That is more photographs than one request can carry.";
const NOT_YOUR_PHOTO: &str =
    "One of those photographs is not in your library, so nothing was attached.";
const BAD_CURSOR: &str = "That page cursor is not one we issued.";
const MINE_REQUIRES_A_SESSION: &str = "Listing your own voyages requires signing in.";

/// All voyage routes.
///
/// `/{id}/photos/order` is registered before nothing in particular — the router
/// matches a literal segment ahead of a parameter, so `PATCH .../photos/order`
/// reaches [`reorder`] and `DELETE .../photos/{uuid}` reaches [`detach`]
/// without the two competing.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/voyages", post(create).get(list))
        .route("/api/voyages/{id}", get(read).patch(update).delete(destroy))
        .route("/api/voyages/{id}/photos", post(attach))
        .route("/api/voyages/{id}/photos/order", patch(reorder))
        .route("/api/voyages/{id}/photos/{photo}", delete(detach))
        .route("/api/voyages/{id}/map", get(map))
        .route("/api/map", get(discover_map))
}

// ---------------------------------------------------------------------------
// The boundary itself
// ---------------------------------------------------------------------------

/// **The only call to [`Granularity::apply`] in the service's HTTP layer.**
///
/// Every coordinate any endpoint in this module emits comes from here, which is
/// what makes "granularity is enforced in one place" a fact about the code
/// rather than a claim in a comment.
///
/// A row with a latitude and no longitude is *unplaced*, not half-placed. Half
/// a coordinate says nothing about where a photograph was taken and there is no
/// honest way to draw it, so it joins the unplaced group rather than becoming a
/// pin at longitude zero.
///
/// Takes the two values rather than a row, so that the voyage row type and the
/// discovery map's own row type share this one call site instead of each
/// growing a `Granularity::apply` of its own.
fn place(
    granularity: Granularity,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> Option<Coordinate> {
    match (latitude, longitude) {
        (Some(latitude), Some(longitude)) => Some(granularity.apply(latitude, longitude)),
        _ => None,
    }
}

/// Parses a granularity as stored, failing **closed**.
///
/// The column has a CHECK constraint, so an unrecognized value means this build
/// is older than the migration that added a fifth step. The safe reading of an
/// unknown precision is the coarsest one available, never `exact`: a rollback
/// to a previous release must not turn every coarse voyage into an exact one
/// for as long as it runs.
fn stored_granularity(stored: &str) -> Granularity {
    match Granularity::parse(stored) {
        Some(granularity) => granularity,
        None => {
            tracing::error!(
                granularity = stored,
                "unknown stored granularity; falling back to the coarsest setting"
            );
            Granularity::Km50
        }
    }
}

/// The label the design prints next to every coordinate — `Exact`, `~1 km`,
/// `~10 km`, `~50 km`.
///
/// Served rather than derived on the client because it appears in three places
/// (the discovery card, the voyage badge, the photo caption) and a viewer who
/// reads a rounded pin as exact is the misreading the whole feature exists to
/// prevent. One spelling, from the server, keeps those three in step.
fn granularity_label(granularity: Granularity) -> &'static str {
    match granularity {
        Granularity::Exact => "Exact",
        Granularity::Km1 => "~1 km",
        Granularity::Km10 => "~10 km",
        Granularity::Km50 => "~50 km",
    }
}

/// The URL a client fetches a variant from. Built from an id and a closed set
/// of three names, so nothing caller-supplied reaches it.
fn variant_url(photo_id: Uuid, kind: VariantKind) -> String {
    format!("/api/photos/{photo_id}/{}", kind.as_str())
}

/// How many distinct points a set of coordinates draws — the "12 places" the
/// detail screen shows beside "48 photos".
///
/// Counted *after* rounding, which is the only count that is true: at ~10 km a
/// day's walk collapses to one cell, and reporting the pre-rounding count would
/// tell a viewer how many distinct spots exist inside a cell. That is a small
/// leak, but it is a leak, and it is free to avoid.
///
/// Compared on the bit patterns because `f64` is not `Eq`. Two coordinates out
/// of one cell are bit-identical — the rounding is deterministic — so this is
/// exact rather than approximate.
fn place_count(coordinates: impl IntoIterator<Item = Coordinate>) -> usize {
    coordinates
        .into_iter()
        .map(|c| (c.latitude().to_bits(), c.longitude().to_bits()))
        .collect::<HashSet<_>>()
        .len()
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// One voyage, with its photographs.
///
/// No `f64` anywhere in it, deliberately: [`VoyagePhotoResponse::coordinate`]
/// is the only field in the whole tree that can carry a latitude, and its type
/// cannot be built without stating a precision.
#[derive(Debug, Serialize)]
struct VoyageResponse {
    id: Uuid,
    owner_id: Uuid,
    title: String,
    description: Option<String>,
    published: bool,
    /// Stored spelling — `exact`, `km1`, `km10`, `km50`. The value a PATCH
    /// sends back.
    granularity: &'static str,
    /// What to print beside a pin, so the client does not invent its own
    /// wording for the precision it is looking at.
    granularity_label: &'static str,
    /// Radius in meters for the area phase 7 draws instead of a pin. `null` at
    /// exact precision, which is the only case where a sharp pin is honest.
    precision_m: Option<u32>,
    created_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
    photo_count: usize,
    /// Photographs that have coordinates, and how many distinct points they
    /// draw once rounded. "48 photos · 12 places" comes from these two.
    placed_count: usize,
    place_count: usize,
    /// The count the design shows in its own card: "6 photos have no location".
    /// They are in `photos` as well — this is for the summary line, not a
    /// substitute for listing them.
    unplaced_count: usize,
    photos: Vec<VoyagePhotoResponse>,
}

/// A photograph inside a voyage.
///
/// An unplaced photograph appears here with `coordinate: null` and
/// `placed: false` rather than being dropped. Absent GPS is ordinary —
/// screenshots, scans, an iPhone with location services off for the day — and
/// the design gives those photographs a visible home at the bottom of the rail.
/// Omitting them would silently shrink a voyage its owner can count.
#[derive(Debug, Serialize)]
struct VoyagePhotoResponse {
    id: Uuid,
    position: i32,
    captured_at: Option<DateTime<Utc>>,
    width: i32,
    height: i32,
    /// `pending`, `ready` or `failed`. A photo exists before its variants do,
    /// so a client must handle `pending` rather than treating the URLs below as
    /// promises.
    variant_status: String,
    thumbnail_url: String,
    display_url: String,
    full_url: String,
    /// `null` for an unplaced photograph. The **only** field in this module's
    /// responses that can carry a latitude, and its type cannot exist without a
    /// granularity having been applied.
    coordinate: Option<Coordinate>,
    placed: bool,
}

/// A discovery card.
///
/// **It carries no location, and that is deliberate rather than an omission.**
/// The design's card shows a cover image, a title, an author and the
/// granularity *label* — the label is the point: a browser scanning the grid
/// can see that one creator publishes exact coordinates and another publishes
/// to ~50 km before the map loads. A "center" coordinate on a card would leak
/// at exactly the precision the detail view does, from the endpoint nobody
/// thinks to check.
///
/// If a later phase does want a center here, it must be an
/// [`Option<Coordinate>`] built through [`place`] — this struct has no `f64`
/// field for it to be added as, which is the point of the shape.
#[derive(Debug, Serialize)]
struct VoyageSummary {
    id: Uuid,
    owner_id: Uuid,
    title: String,
    description: Option<String>,
    published: bool,
    granularity: &'static str,
    granularity_label: &'static str,
    precision_m: Option<u32>,
    created_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    voyages: Vec<VoyageSummary>,
    /// What to pass as `before` for the next page, or `null` at the end.
    next_before: Option<String>,
}

/// What a map needs and nothing else (T9).
///
/// Phase 7 fetches this for every pin on screen, so it deliberately does not
/// carry the detail payload: no dimensions, no capture time, no variant status,
/// no description. A pin is a point, an id and an image — sending the rest
/// multiplies the cost of the most-visited endpoint by everything the map will
/// not draw.
#[derive(Debug, Serialize)]
struct MapResponse {
    voyage_id: Uuid,
    granularity: &'static str,
    granularity_label: &'static str,
    /// Repeated here as well as on every pin because the map needs it to size
    /// its areas even when the voyage has no pins at all.
    precision_m: Option<u32>,
    pins: Vec<MapPin>,
    /// The headline the design prints beside the map: "6 photos have no
    /// location".
    unplaced_count: usize,
    /// And what its "View" button opens. Listed rather than merely counted
    /// because a photograph that disappears from a response is, to its owner,
    /// indistinguishable from one that was lost — the map is an endpoint they
    /// look at, so it has to account for every photograph in the voyage even
    /// though it can only draw some of them.
    unplaced: Vec<MapUnplaced>,
}

/// One pin. A pin has a coordinate by definition — an unplaced photograph is
/// listed below, not drawn — so this is [`Coordinate`] rather than an option,
/// and the type makes "a pin with no location" unrepresentable.
#[derive(Debug, Serialize)]
struct MapPin {
    photo_id: Uuid,
    coordinate: Coordinate,
    thumbnail_url: String,
}

/// A photograph in the voyage that the map cannot draw.
#[derive(Debug, Serialize)]
struct MapUnplaced {
    photo_id: Uuid,
    thumbnail_url: String,
    /// Always `false`. Stated rather than implied by the array it sits in,
    /// because a client that flattens the two lists — which is what viewing the
    /// unplaced group does — must not lose the flag on the way.
    placed: bool,
}

impl VoyageResponse {
    /// Builds the detail payload. Takes the row types, which are not
    /// `Serialize`, and is the only thing that turns them into something that
    /// is.
    fn build(voyage: Voyage, photos: Vec<VoyagePhoto>) -> Self {
        let granularity = stored_granularity(&voyage.granularity);

        let photos: Vec<VoyagePhotoResponse> = photos
            .into_iter()
            .map(|photo| {
                let coordinate = place(granularity, photo.latitude, photo.longitude);
                VoyagePhotoResponse {
                    id: photo.photo_id,
                    position: photo.position,
                    captured_at: photo.captured_at,
                    width: photo.width,
                    height: photo.height,
                    variant_status: photo.variant_status,
                    thumbnail_url: variant_url(photo.photo_id, VariantKind::Thumb),
                    display_url: variant_url(photo.photo_id, VariantKind::Display),
                    full_url: variant_url(photo.photo_id, VariantKind::Full),
                    placed: coordinate.is_some(),
                    coordinate,
                }
            })
            .collect();

        let placed_count = photos.iter().filter(|photo| photo.placed).count();
        let place_count = place_count(photos.iter().filter_map(|photo| photo.coordinate));

        Self {
            id: voyage.id,
            owner_id: voyage.owner_id,
            title: voyage.title,
            description: voyage.description,
            published: voyage.published,
            granularity: granularity.as_str(),
            granularity_label: granularity_label(granularity),
            precision_m: granularity.display_radius_m(),
            created_at: voyage.created_at,
            published_at: voyage.published_at,
            photo_count: photos.len(),
            placed_count,
            place_count,
            unplaced_count: photos.len() - placed_count,
            photos,
        }
    }
}

impl From<Voyage> for VoyageSummary {
    fn from(voyage: Voyage) -> Self {
        let granularity = stored_granularity(&voyage.granularity);

        Self {
            id: voyage.id,
            owner_id: voyage.owner_id,
            title: voyage.title,
            description: voyage.description,
            published: voyage.published,
            granularity: granularity.as_str(),
            granularity_label: granularity_label(granularity),
            precision_m: granularity.display_radius_m(),
            created_at: voyage.created_at,
            published_at: voyage.published_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Create, update, delete
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateRequest {
    /// Optional, because a voyage starts as a draft and the design's edit
    /// screen exists to fill it in. It becomes required at the moment of
    /// publishing and only then — see [`publishable`].
    title: Option<String>,
    description: Option<String>,
    /// Stored spelling. Absent means the schema default, `exact`.
    granularity: Option<String>,
}

/// Creates a draft (T8).
///
/// Always a draft, whatever the request says: a voyage is world-readable the
/// instant it is published (D6), and a half-populated one that arrived
/// published by default would be readable before its author had seen it. There
/// is no `published` field on this request for that reason — publishing is a
/// PATCH, which is a second deliberate act.
async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<VoyageResponse>), Rejection> {
    let title = clean_title(body.title.as_deref())?;
    let description = clean_description(body.description.as_deref())?;
    let granularity = match body.granularity.as_deref() {
        Some(spelling) => parse_granularity(spelling)?,
        None => Granularity::Exact,
    };

    let voyage = repo::create(
        &state.pool,
        &NewVoyage {
            owner_id: user.user_id,
            title,
            description,
            granularity: granularity.as_str().to_owned(),
        },
    )
    .await?;

    // A new voyage has no photographs, so no second query: the empty vector is
    // the truth rather than a shortcut.
    Ok((
        StatusCode::CREATED,
        Json(VoyageResponse::build(voyage, Vec::new())),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct UpdateRequest {
    title: Option<String>,
    /// Three states, not two. Absent leaves the description alone; `null`
    /// clears it; a string sets it. A plain `Option<String>` cannot tell the
    /// first two apart, which would make "remove my description" impossible to
    /// express.
    #[serde(default, deserialize_with = "present")]
    description: Option<Option<String>>,
    granularity: Option<String>,
    published: Option<bool>,
}

/// Distinguishes an absent JSON field from an explicit `null`.
///
/// `Option<Option<T>>` alone does not: serde folds `null` into the outer
/// `None`, which is the same value an absent field produces. Deserializing the
/// inner option and wrapping it is the standard way to keep the two apart.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Edits a voyage, owner only.
///
/// The read before the write is not an authorization check — the repository's
/// update carries the ownership predicate in its `WHERE` clause, so there is no
/// window between the two — it is there to compute the *effective* title and
/// published flag, since publishing without changing the title has to validate
/// against the title already stored.
async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRequest>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let current = owned(&state, &user, id).await?;

    let title = match body.title.as_deref() {
        Some(title) => Some(clean_title(Some(title))?),
        None => None,
    };
    let description = match body.description.as_ref() {
        Some(value) => Some(clean_description(value.as_deref())?),
        None => None,
    };
    let granularity = match body.granularity.as_deref() {
        Some(spelling) => Some(parse_granularity(spelling)?),
        None => None,
    };

    // The design's only publish precondition, checked against the state the
    // voyage will be in after this patch rather than the state it is in now —
    // "set the title and publish" is one request, and clearing the title of an
    // already-published voyage has to fail for the same reason.
    let effective_title = title.as_deref().unwrap_or(&current.title);
    let effective_published = body.published.unwrap_or(current.published);
    publishable(effective_title, effective_published)?;

    let patch = VoyagePatch {
        title,
        description,
        granularity: granularity.map(|value| value.as_str().to_owned()),
        published: body.published,
    };

    // `Ok(None)` is "no such voyage, or not yours" and cannot be reached here,
    // since `owned` has already answered both — but a concurrent delete makes
    // it possible, and 404 is the honest answer to that race too.
    let voyage = repo::update(&state.pool, id, user.user_id, &patch)
        .await?
        .ok_or(AppError::NotFound)?;

    let photos = repo::photos_for_voyage(&state.pool, id).await?;

    Ok(Json(VoyageResponse::build(voyage, photos)))
}

/// Deletes a voyage, owner only.
///
/// The photographs survive: the join rows cascade and the photos do not, so
/// deleting a voyage returns its images to the owner's library. Someone tidying
/// up a draft is not asking to destroy the photographs in it.
async fn destroy(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Rejection> {
    if repo::delete(&state.pool, id, user.user_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Not yours and not there are one answer, from one statement. A 403
        // here would confirm that a draft exists at this id.
        Err(AppError::NotFound.into())
    }
}

// ---------------------------------------------------------------------------
// Photos in a voyage
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PhotoIdsRequest {
    photo_ids: Vec<Uuid>,
}

/// Attaches photographs the caller owns (T3).
///
/// The ownership predicate lives inside the repository's insert, so there is no
/// check-then-write window for a caller to race. What this handler adds is the
/// **atomicity** the acceptance criterion asks for: "attaching another user's
/// photo returns 403 and attaches nothing", so the insert runs inside a
/// transaction that is rolled back the moment the row count says something was
/// refused.
///
/// A short count has two causes — a photograph that is not the caller's, and
/// one that is already in this voyage — and the insert cannot report which.
/// They are told apart on the failure path only, which is the rare one, by
/// re-reading the named photographs inside the same transaction. That costs a
/// query per named photograph when a request is already failing and nothing at
/// all when it succeeds.
///
/// **The 403 here is not the enumeration oracle 404 exists to avoid.** The
/// caller already owns the voyage, so its existence is not news; and a
/// photograph that does not exist produces the same 403 as one owned by
/// somebody else, so the status distinguishes nothing about the id.
async fn attach(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PhotoIdsRequest>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let photo_ids = clean_photo_ids(body.photo_ids)?;

    let voyage = owned(&state, &user, id).await?;

    let mut tx = state.pool.begin().await?;

    let added = repo::attach_photos(&mut *tx, id, user.user_id, &photo_ids).await?;

    if added as usize != photo_ids.len() {
        for photo_id in &photo_ids {
            let mine = photos_repo::find_by_id(&mut *tx, *photo_id)
                .await?
                .is_some_and(|photo| photo.owner_id == user.user_id);

            if !mine {
                // Explicit rather than a drop, so the rollback has completed
                // before the response says nothing was attached.
                tx.rollback().await?;
                return Err(Rejection::forbidden(NOT_YOUR_PHOTO));
            }
        }
        // Every named photograph is the caller's, so the shortfall was
        // photographs already in this voyage. Attaching a photo twice is not an
        // error and must not become one — the edit screen cannot tell a
        // half-finished request from an unsent one, so "it is attached" is the
        // honest answer.
    }

    tx.commit().await?;

    let photos = repo::photos_for_voyage(&state.pool, id).await?;

    Ok(Json(VoyageResponse::build(voyage, photos)))
}

/// Removes one photograph from a voyage. The photograph itself is untouched.
async fn detach(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, photo_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Rejection> {
    if repo::detach_photo(&state.pool, id, user.user_id, photo_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound.into())
    }
}

/// Sets an explicit order (T4).
///
/// The body is the desired sequence, not a delta: a partial order would need a
/// rule for where the unnamed photographs go, and every such rule is one the
/// client and the server can disagree about.
async fn reorder(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PhotoIdsRequest>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let photo_ids = clean_photo_ids(body.photo_ids)?;

    // `reorder` returns no indication of whether it matched anything, so the
    // 404 for "not yours or not there" has to be answered here.
    let voyage = owned(&state, &user, id).await?;

    repo::reorder(&state.pool, id, user.user_id, &photo_ids).await?;

    let photos = repo::photos_for_voyage(&state.pool, id).await?;

    Ok(Json(VoyageResponse::build(voyage, photos)))
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// One voyage, at the precision its owner chose (T5, T6).
///
/// Public when published — no session required, because every voyage is public
/// (D6) and a shared link has to open for someone who has never signed in.
///
/// Two queries regardless of photo count (T6/T9): the voyage, then all of its
/// photographs in one statement. A loop of `await`s over photo ids looks
/// concurrent and is not, and the cost would scale with photo count on the
/// product's most-visited endpoint.
async fn read(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Path(id): Path<Uuid>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let voyage = visible(&state, user.as_ref(), id).await?;
    let photos = repo::photos_for_voyage(&state.pool, id).await?;

    Ok(Json(VoyageResponse::build(voyage, photos)))
}

/// The map payload (T9).
///
/// Same visibility rule and same granularity as [`read`] — this is the endpoint
/// it would be easiest to leave out of the check, and it is the one phase 7
/// calls most.
async fn map(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Path(id): Path<Uuid>,
) -> Result<Json<MapResponse>, Rejection> {
    let voyage = visible(&state, user.as_ref(), id).await?;
    let granularity = stored_granularity(&voyage.granularity);

    let photos = repo::photos_for_voyage(&state.pool, id).await?;

    let mut pins = Vec::with_capacity(photos.len());
    let mut unplaced = Vec::new();

    for photo in &photos {
        let thumbnail_url = variant_url(photo.photo_id, VariantKind::Thumb);

        match place(granularity, photo.latitude, photo.longitude) {
            Some(coordinate) => pins.push(MapPin {
                photo_id: photo.photo_id,
                coordinate,
                thumbnail_url,
            }),
            None => unplaced.push(MapUnplaced {
                photo_id: photo.photo_id,
                thumbnail_url,
                placed: false,
            }),
        }
    }

    Ok(Json(MapResponse {
        voyage_id: voyage.id,
        granularity: granularity.as_str(),
        granularity_label: granularity_label(granularity),
        precision_m: granularity.display_radius_m(),
        unplaced_count: unplaced.len(),
        pins,
        unplaced,
    }))
}

// ---------------------------------------------------------------------------
// The discovery map
// ---------------------------------------------------------------------------

/// What the discovery map asks for: a rectangle and how far in the camera is.
#[derive(Debug, Deserialize)]
struct DiscoverMapQuery {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    /// Web Mercator zoom. Decides how many pixels apart two photographs are,
    /// and therefore whether they are drawn as one marker or two.
    zoom: f64,
}

/// A group of photographs the discovery map draws as one marker.
#[derive(Debug, Serialize)]
struct DiscoverCluster {
    /// **A real photograph's published position**, not a computed center.
    ///
    /// That is a deliberate constraint rather than a convenience. [`Coordinate`]
    /// has private fields and one constructor, so the only way to put a
    /// latitude in a response is to have rounded it through a granularity —
    /// and a centroid averaged from several photographs is not the output of
    /// that constructor. Taking the representative's own coordinate keeps
    /// [`place`] the single call to `Granularity::apply` in this module, and
    /// means every position the map draws is one a voyage already publishes.
    coordinate: Coordinate,
    /// How many photographs the marker stands for, the representative
    /// included.
    count: usize,
    /// The representative: the lowest photo id in the group, so the choice is
    /// deterministic and the same rectangle draws the same markers twice
    /// running.
    photo_id: Uuid,
    /// Which published voyage the representative belongs to, so a marker can be
    /// followed to something.
    voyage_id: Uuid,
    thumbnail_url: String,
}

#[derive(Debug, Serialize)]
struct DiscoverMapResponse {
    clusters: Vec<DiscoverCluster>,
    /// True when there were more photographs in the rectangle than
    /// [`DISCOVER_MAP_LIMIT`] and some were left out.
    ///
    /// Stated rather than hidden. A map that silently drops half a continent
    /// looks exactly like a map of a continent with fewer photographs in it,
    /// and the reader has no way to tell which they are looking at.
    truncated: bool,
}

/// The most photographs one discovery-map request will consider.
///
/// A cap is unavoidable: at the world view the rectangle is the whole planet,
/// and the answer would otherwise be every published photograph in the
/// database. Rows past the cap are dropped rather than merged, which is why
/// exceeding it is reported.
const DISCOVER_MAP_LIMIT: i64 = 20_000;

/// How far apart, in screen pixels, two photographs must be to get their own
/// markers.
///
/// The 44px touch target from `design/tokens.css`, the same figure the browser
/// clusters a single voyage by. Two markers closer than one touch target cannot
/// be tapped apart, so leaving them separate produces taps that appear to do
/// nothing.
const DISCOVER_CLUSTER_CELL_PX: f64 = 44.0;

/// The widest granularity cell, in degrees of latitude — 50 km over 111.32 km
/// per degree.
///
/// Used to widen the queried rectangle. Rounding moves a photograph by up to
/// half a cell, so a photograph just outside the requested rectangle can round
/// to a position inside it; querying only the exact rectangle would drop it and
/// leave a visible gap at the edge of every pan.
const MAX_CELL_DEG_LAT: f64 = 50.0 / 111.32;

/// The same widening for longitude.
///
/// Longitude cells grow as `1 / cos(latitude)` and the granularity boundary
/// freezes that scaling at 85°, so this is the widest a cell ever gets:
/// about 5.2°.
const MAX_CELL_DEG_LON: f64 = MAX_CELL_DEG_LAT / 0.087_155_742_7; // cos(85°)

/// Photographs from every **published** voyage, clustered, inside a rectangle.
///
/// This is the endpoint the phase 7 goal means by "clustering is always
/// server-side", and here it genuinely has to be: the rectangle at a world view
/// covers every published photograph there is, and no client can be sent them
/// all to group for itself. A single voyage is the opposite case — it fits in
/// one response, and `GET /api/voyages/{id}/map` sends it whole.
///
/// No session is required and none is consulted. Only published voyages are
/// considered, so the answer is the same for everyone, which is also what makes
/// it cacheable.
///
/// # What is rounded, and when
///
/// Each photograph is rounded to **its own voyage's** granularity before
/// anything else happens — the same [`place`] call every other endpoint here
/// uses. Clustering then groups already-published positions. Doing it the other
/// way round, grouping exact coordinates and rounding the group, would make the
/// marker a function of coordinates that were never allowed out.
async fn discover_map(
    State(state): State<AppState>,
    Query(query): Query<DiscoverMapQuery>,
) -> Result<Json<DiscoverMapResponse>, Rejection> {
    let bbox = Bbox::parse(&query)?;

    let rows = repo::published_photos_in_bbox(
        &state.pool,
        to_degrees(bbox.south - MAX_CELL_DEG_LAT, -90.0, 90.0),
        to_degrees(bbox.north + MAX_CELL_DEG_LAT, -90.0, 90.0),
        to_degrees(bbox.west - MAX_CELL_DEG_LON, -180.0, 180.0),
        to_degrees(bbox.east + MAX_CELL_DEG_LON, -180.0, 180.0),
        bbox.wraps,
        // One more than the cap, so that hitting it is detectable.
        DISCOVER_MAP_LIMIT + 1,
    )
    .await?;

    let truncated = rows.len() as i64 > DISCOVER_MAP_LIMIT;
    let considered = &rows[..rows.len().min(DISCOVER_MAP_LIMIT as usize)];

    if truncated {
        tracing::warn!(
            limit = DISCOVER_MAP_LIMIT,
            zoom = bbox.zoom,
            "discovery map hit its row cap; some photographs were left out"
        );
    }

    Ok(Json(DiscoverMapResponse {
        clusters: cluster_for_discovery(considered, bbox.zoom),
        truncated,
    }))
}

/// A validated rectangle.
struct Bbox {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    zoom: f64,
    /// True when the rectangle crosses the antimeridian, so `west` is
    /// numerically greater than `east`.
    wraps: bool,
}

const BAD_BOUNDS: &str = "That is not a rectangle we can look in.";
const BAD_ZOOM: &str = "That is not a zoom level we recognize.";

/// The zoom levels the map can be at. Zero is the whole world in one tile; the
/// tile source stops at 19 and the client allows one blurry level past it.
const MAX_QUERY_ZOOM: f64 = 20.0;

impl Bbox {
    fn parse(query: &DiscoverMapQuery) -> Result<Self, Rejection> {
        // Non-finite values first, and explicitly.
        //
        // The range checks below happen to catch NaN already, because
        // `RangeInclusive::contains` compares with `PartialOrd` and every
        // comparison against NaN is false — so `!contains(&NaN)` is true and the
        // value is refused. That is a subtlety rather than a design. Written the
        // other obvious way, `south < -90.0 || south > 90.0`, both halves are
        // false for NaN and it sails through, to become a pixel coordinate with
        // no grid cell. This loop is what makes the refusal survive that
        // rewrite.
        for value in [query.west, query.south, query.east, query.north] {
            if !value.is_finite() {
                return Err(Rejection::field("bounds", BAD_BOUNDS));
            }
        }

        if !query.zoom.is_finite() || query.zoom < 0.0 || query.zoom > MAX_QUERY_ZOOM {
            return Err(Rejection::field("zoom", BAD_ZOOM));
        }

        if !(-90.0..=90.0).contains(&query.south)
            || !(-90.0..=90.0).contains(&query.north)
            || !(-180.0..=180.0).contains(&query.west)
            || !(-180.0..=180.0).contains(&query.east)
            || query.south > query.north
        {
            return Err(Rejection::field("bounds", BAD_BOUNDS));
        }

        Ok(Bbox {
            west: query.west,
            south: query.south,
            east: query.east,
            north: query.north,
            zoom: query.zoom,
            // West greater than east is how a rectangle crossing the
            // antimeridian arrives — from Fiji to Samoa the numbers run 177 to
            // -172. It is not an error and it is not an empty rectangle.
            wraps: query.west > query.east,
        })
    }
}

/// Clamps and converts to the column's type.
///
/// `numeric` rather than `float8` so the comparison can use
/// `photos_location_idx`; casting the column instead would make the index
/// unusable. A value that will not convert clamps to the bound, which is
/// correct for a rectangle edge — the widest sensible rectangle is the whole
/// world.
fn to_degrees(value: f64, low: f64, high: f64) -> rust_decimal::Decimal {
    let clamped = value.clamp(low, high);
    rust_decimal::Decimal::from_f64_retain(clamped)
        .map(|decimal| decimal.round_dp(6))
        .unwrap_or_else(|| {
            rust_decimal::Decimal::from_f64_retain(low).unwrap_or(rust_decimal::Decimal::ZERO)
        })
}

/// Groups already-rounded positions into one marker per grid cell.
///
/// A fixed grid rather than the greedy nearest-neighbor grouping the browser
/// uses for a single voyage. Both are defensible; the grid is chosen here
/// because it is linear in the number of photographs rather than quadratic, and
/// this endpoint's input is bounded by [`DISCOVER_MAP_LIMIT`] rather than by
/// how many photographs one person put in one voyage. The visible cost is that
/// two photographs either side of a cell boundary stay apart when they are
/// close enough to overlap; zooming in separates them anyway, and at the scale
/// this endpoint is read at the difference is not noticeable.
///
/// The grid is anchored at zero in world-pixel space, so it is the same grid
/// for every request at a given zoom. A grid anchored on the rectangle would
/// regroup the same photographs differently on every pan, and markers would
/// visibly reshuffle as the map moved.
fn cluster_for_discovery(rows: &[repo::DiscoverPhoto], zoom: f64) -> Vec<DiscoverCluster> {
    // Insertion-ordered, so the output follows the input's `ORDER BY photo_id`
    // and the response is byte-identical for an unchanged rectangle.
    let mut cells: Vec<((i64, i64), DiscoverCluster)> = Vec::new();
    let mut index: HashMap<(i64, i64), usize> = HashMap::new();

    for row in rows {
        let granularity = stored_granularity(&row.granularity);

        // Rounded before it is grouped. The grid never sees an exact
        // coordinate from a voyage that does not publish one.
        let Some(coordinate) = place(granularity, Some(row.latitude), Some(row.longitude)) else {
            continue;
        };

        let (x, y) = world_pixels(coordinate.latitude(), coordinate.longitude(), zoom);
        let key = (
            (x / DISCOVER_CLUSTER_CELL_PX).floor() as i64,
            (y / DISCOVER_CLUSTER_CELL_PX).floor() as i64,
        );

        match index.get(&key) {
            Some(&at) => cells[at].1.count += 1,
            None => {
                index.insert(key, cells.len());
                cells.push((
                    key,
                    DiscoverCluster {
                        coordinate,
                        count: 1,
                        photo_id: row.photo_id,
                        voyage_id: row.voyage_id,
                        thumbnail_url: variant_url(row.photo_id, VariantKind::Thumb),
                    },
                ));
            }
        }
    }

    cells.into_iter().map(|(_, cluster)| cluster).collect()
}

/// Web Mercator, in world pixels at `zoom`.
///
/// The whole world is `256 * 2^zoom` pixels across, which is the same
/// convention MapLibre uses, so a distance computed here is the distance the
/// browser will draw. Written out rather than shared with the frontend because
/// the two are separate programs; both are checked against the same published
/// constants.
fn world_pixels(latitude: f64, longitude: f64, zoom: f64) -> (f64, f64) {
    /// The latitude Web Mercator stops at. Past it the projection runs to
    /// infinity, and an infinite pixel coordinate has no grid cell.
    const MAX_MERCATOR_LATITUDE: f64 = 85.051_129;

    let scale = 256.0 * 2f64.powf(zoom);
    let clamped = latitude.clamp(-MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
    let sin = clamped.to_radians().sin();

    (
        ((longitude + 180.0) / 360.0) * scale,
        (0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * std::f64::consts::PI)) * scale,
    )
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    /// The caller's own voyages, drafts included, instead of the public feed.
    /// Requires a session; the feed itself does not.
    #[serde(default)]
    mine: bool,
    /// Cursor. A published-at timestamp for the feed, a voyage id for `mine` —
    /// a string here because the two modes key on different columns, and it is
    /// opaque to the caller either way.
    before: Option<String>,
    limit: Option<i64>,
}

/// The discovery feed, and the caller's own list.
///
/// Cursor rather than offset pagination (T7): a voyage published between two
/// requests shifts every offset by one, so a page boundary skips or repeats a
/// voyage — and on a feed that is actively being published to that is the
/// common case, not the rare one. Keyset on the column the ordering uses costs
/// an index seek however deep the page is.
async fn list(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListResponse>, Rejection> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);

    let voyages = if query.mine {
        // Drafts are in this list, so it is the one listing that needs a
        // session. The public feed above it does not.
        let Some(user) = user else {
            tracing::debug!(reason = MINE_REQUIRES_A_SESSION, "listing rejected");
            return Err(AppError::Unauthorized.into());
        };

        let before = match query.before.as_deref() {
            Some(cursor) => Some(Uuid::parse_str(cursor).map_err(|_| bad_cursor())?),
            None => None,
        };

        repo::list_for_owner(&state.pool, user.user_id, before, limit).await?
    } else {
        let before = match query.before.as_deref() {
            Some(cursor) => Some(
                DateTime::parse_from_rfc3339(cursor)
                    .map_err(|_| bad_cursor())?
                    .with_timezone(&Utc),
            ),
            None => None,
        };

        repo::list_published(&state.pool, before, limit).await?
    };

    // A full page implies there may be more; a short one is the end. Cheaper
    // than a count, and a count would be stale the moment it was taken.
    let next_before = (voyages.len() as i64 == limit)
        .then(|| {
            voyages.last().and_then(|voyage| {
                if query.mine {
                    Some(voyage.id.to_string())
                } else {
                    voyage.published_at.map(|at| at.to_rfc3339())
                }
            })
        })
        .flatten();

    Ok(Json(ListResponse {
        voyages: voyages.into_iter().map(VoyageSummary::from).collect(),
        next_before,
    }))
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Loads a voyage the caller may read.
///
/// A draft is **404** to everyone but its owner, never 403 (T6). "Forbidden"
/// confirms that a voyage exists at that id, which turns id probing into an
/// enumeration oracle for unpublished work — and unpublished work is precisely
/// what somebody probing would be looking for.
async fn visible(
    state: &AppState,
    user: Option<&CurrentUser>,
    id: Uuid,
) -> Result<Voyage, Rejection> {
    let voyage = repo::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let readable = voyage.published || user.is_some_and(|user| user.user_id == voyage.owner_id);

    if !readable {
        return Err(AppError::NotFound.into());
    }

    Ok(voyage)
}

/// Loads a voyage the caller owns, for the mutations that need to read before
/// they write.
///
/// Also 404 rather than 403, and for the same reason: a stranger PATCHing an id
/// must not learn from the status whether anything is there.
async fn owned(state: &AppState, user: &CurrentUser, id: Uuid) -> Result<Voyage, Rejection> {
    let voyage = repo::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;

    if voyage.owner_id != user.user_id {
        return Err(AppError::NotFound.into());
    }

    Ok(voyage)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// The design's only publish precondition.
///
/// A field-level error rather than a generic one, because the edit screen
/// attaches the message to the title input. A banner saying "something is
/// wrong" on a form with four fields is a worse answer than a sentence beside
/// the one that is.
fn publishable(title: &str, published: bool) -> Result<(), Rejection> {
    if published && title.trim().is_empty() {
        return Err(Rejection::field("title", TITLE_REQUIRED));
    }
    Ok(())
}

/// Trims and bounds a title. Absent becomes empty, which is a legal state for a
/// draft and an illegal one for a published voyage — see [`publishable`].
fn clean_title(title: Option<&str>) -> Result<String, Rejection> {
    let title = title.unwrap_or_default().trim();

    // Counted in characters rather than bytes: a limit that rejects a Japanese
    // title a third the length of an English one is a bug, not a policy.
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(Rejection::field("title", TITLE_TOO_LONG));
    }

    Ok(title.to_owned())
}

/// Trims and bounds a description. A description that is blank after trimming
/// is `None`, so "  " and "" and absent all mean the same stored value rather
/// than three that render differently.
fn clean_description(description: Option<&str>) -> Result<Option<String>, Rejection> {
    let Some(description) = description else {
        return Ok(None);
    };
    let description = description.trim();

    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(Rejection::field("description", DESCRIPTION_TOO_LONG));
    }

    Ok((!description.is_empty()).then(|| description.to_owned()))
}

fn parse_granularity(spelling: &str) -> Result<Granularity, Rejection> {
    Granularity::parse(spelling).ok_or_else(|| Rejection::field("granularity", UNKNOWN_GRANULARITY))
}

/// De-duplicates and bounds a list of photo ids.
///
/// De-duplication first, because the attach path compares the number of rows
/// inserted against the number requested: a caller who names the same
/// photograph twice would otherwise look exactly like one attaching a
/// photograph they do not own.
fn clean_photo_ids(photo_ids: Vec<Uuid>) -> Result<Vec<Uuid>, Rejection> {
    if photo_ids.len() > MAX_PHOTO_IDS {
        return Err(Rejection::field("photo_ids", TOO_MANY_PHOTOS));
    }

    let mut seen = HashSet::with_capacity(photo_ids.len());
    // Order is preserved, because for a reorder the order *is* the request.
    let photo_ids: Vec<Uuid> = photo_ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();

    if photo_ids.is_empty() {
        return Err(Rejection::field("photo_ids", NO_PHOTOS_NAMED));
    }

    Ok(photo_ids)
}

fn bad_cursor() -> Rejection {
    AppError::BadRequest(BAD_CURSOR.to_owned()).into()
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

/// A failure from one of these handlers.
///
/// Two things [`AppError`] cannot express and this phase needs: a 422 carrying
/// the **name of the field** that failed, which is what lets the edit screen
/// put "A title is required before publishing" beside the title input rather
/// than in a banner; and a 403 whose message is chosen at the call site.
///
/// Every message is a `&'static str`. That is not a style preference — it is
/// how "an error must never echo a coordinate" is enforced. There is no
/// `String` for a `format!` to build one into.
#[derive(Debug)]
enum Rejection {
    App(AppError),
    /// 422 with the offending field named.
    Field {
        field: &'static str,
        message: &'static str,
    },
    Forbidden {
        message: &'static str,
    },
}

impl Rejection {
    fn field(field: &'static str, message: &'static str) -> Self {
        Rejection::Field { field, message }
    }

    fn forbidden(message: &'static str) -> Self {
        Rejection::Forbidden { message }
    }
}

impl From<AppError> for Rejection {
    fn from(err: AppError) -> Self {
        Rejection::App(err)
    }
}

impl From<sqlx::Error> for Rejection {
    fn from(err: sqlx::Error) -> Self {
        Rejection::App(AppError::Database(err))
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            // `AppError` logs and renders itself; there is nothing to add.
            Rejection::App(err) => err.into_response(),

            // The body keeps `code` and `message` so a client that only knows
            // the common error shape still works, and adds `errors` for one
            // that wants to attach the message to an input.
            Rejection::Field { field, message } => {
                tracing::debug!(field, "request rejected");

                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "code": "validation_failed",
                        "message": message,
                        "errors": [{ "field": field, "message": message }],
                    })),
                )
                    .into_response()
            }

            Rejection::Forbidden { message } => {
                tracing::debug!("request rejected as forbidden");

                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "code": "forbidden", "message": message })),
                )
                    .into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request;
    use secrecy::SecretString;
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
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

    /// The fallback direction is the whole point. A build older than the
    /// migration that added a fifth precision step must read the unknown value
    /// as the *coarsest* setting — reading it as `exact` would publish every
    /// coarse voyage's true coordinates for as long as the rollback lasted.
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

    /// The shape of a photo in a response, pinned.
    ///
    /// This is a regression guard on the one rule rather than a formality: the
    /// assertion fails the moment somebody adds a `latitude` field to the DTO,
    /// which is the shortcut the whole design is arranged to prevent. `place`
    /// is the only route to the `coordinate` key, and `Coordinate` is the only
    /// type that fits it.
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

    /// The endpoint nobody thinks to check. A discovery card carries the
    /// precision *label* and no location whatsoever — the label is what tells a
    /// browser what they are about to look at, and a center coordinate would
    /// leak at exactly the precision the detail view does.
    #[test]
    fn a_discovery_card_carries_a_precision_label_and_no_location() {
        let json =
            serde_json::to_value(VoyageSummary::from(a_voyage("km10", true))).expect("serializes");

        assert_eq!(
            keys(&json),
            vec![
                "created_at",
                "description",
                "granularity",
                "granularity_label",
                "id",
                "owner_id",
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

    /// The map payload is lean on purpose: phase 7 fetches it for every pin on
    /// screen, so anything the map will not draw is bandwidth spent per pin.
    #[test]
    fn the_map_payload_carries_only_what_a_map_draws() {
        let voyage = a_voyage("km10", true);
        let granularity = stored_granularity(&voyage.granularity);
        let photo = a_photo();

        let response = MapResponse {
            voyage_id: voyage.id,
            granularity: granularity.as_str(),
            granularity_label: granularity_label(granularity),
            precision_m: granularity.display_radius_m(),
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

        // And the detail payload carries strictly more, which is the difference
        // the two endpoints exist to express: a pin is a point, an id and an
        // image, and everything below is weight the map would pay per pin.
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

    #[tokio::test]
    async fn an_unknown_precision_is_a_field_error_on_the_granularity() {
        assert_eq!(parse_granularity("km10").expect("known"), Granularity::Km10);

        let (status, body) = rendered(parse_granularity("km7").expect_err("unknown")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["field"], "granularity");
    }

    /// De-duplication is load-bearing rather than tidy: the attach path decides
    /// whether a photograph was refused by comparing the number of rows
    /// inserted with the number requested, and a caller who names one photo
    /// twice would otherwise be answered with a 403.
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

    /// T3's status. A 403 here is safe where a 403 on a voyage would not be:
    /// the caller already owns the voyage, and a photograph that does not exist
    /// produces the same answer as one owned by somebody else.
    #[tokio::test]
    async fn a_foreign_photo_is_a_403_that_distinguishes_nothing() {
        let (status, body) = rendered(Rejection::forbidden(NOT_YOUR_PHOTO)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "forbidden");
        assert_eq!(body["message"], NOT_YOUR_PHOTO);
    }

    // -- request parsing -----------------------------------------------------

    /// Absent, `null` and a value are three different intentions, and a PATCH
    /// that could not tell the first two apart would make "remove my
    /// description" impossible to express.
    #[test]
    fn a_patch_tells_an_absent_description_from_an_explicit_null() {
        let absent: UpdateRequest = serde_json::from_str("{}").expect("valid");
        assert_eq!(absent.description, None);

        let cleared: UpdateRequest =
            serde_json::from_str(r#"{"description":null}"#).expect("valid");
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

    // -- the central check ---------------------------------------------------
    //
    // These depend on `Granularity::apply`, which is owned by the granularity
    // agent and is `todo!()` at the time of writing. They are the phase's T5
    // criterion and are written against the fixed signature, so they start
    // passing the moment that lands.

    /// T5. The exact stored coordinate appears in **no** response body of any
    /// endpoint once the voyage is set coarser than exact.
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
                precision_m: granularity.display_radius_m(),
                pins: vec![MapPin {
                    photo_id: photo.photo_id,
                    coordinate: place(granularity, photo.latitude, photo.longitude)
                        .expect("placed"),
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

        let json =
            serde_json::to_value(VoyageResponse::build(a_voyage("exact", true), vec![photo]))
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
        let twice =
            serde_json::to_string(&VoyageResponse::build(a_voyage("km10", true), vec![photo]))
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
}
