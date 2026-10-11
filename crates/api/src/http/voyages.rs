//! Voyage endpoints.
//!
//! ```text
//! POST   /api/voyages                       create (draft)
//! GET    /api/voyages                       discovery feed, public
//! GET    /api/voyages/{id}                  read; public if published
//! PATCH  /api/voyages/{id}                  owner only
//! DELETE /api/voyages/{id}                  owner only
//! DELETE /api/voyages/{id}/photos/{photo}   delete photo
//! PATCH  /api/voyages/{id}/photos/order     reorder
//! GET    /api/voyages/{id}/map              rounded map payload
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

use std::collections::HashSet;
mod discovery;

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
        .route("/api/voyages/{id}/photos/order", patch(reorder))
        .route("/api/voyages/{id}/photos/{photo}", delete(detach))
        .route("/api/voyages/{id}/map", get(map))
        .route("/api/map", get(discovery::discover_map))
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
        (Some(latitude), Some(longitude)) => Some(place_pair(granularity, latitude, longitude)),
        _ => None,
    }
}

fn place_pair(granularity: Granularity, latitude: f64, longitude: f64) -> Coordinate {
    granularity.apply(latitude, longitude)
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
    /// Per-axis half-width in meters for the public rounding cell. `null` at
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

/// A discovery card with a cover, author, and granularity label. It contains
/// no coordinates; map and detail responses apply `Granularity` explicitly.
#[derive(Debug, Serialize)]
pub(crate) struct VoyageSummary {
    /// First ready variant in voyage order; never an original upload.
    cover_url: Option<String>,
    photos_pending: bool,
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
/// The map fetches this for every pin on screen, so it deliberately does not
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
            precision_m: granularity.half_cell_m(),
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
            cover_url: None,
            photos_pending: false,
            id: voyage.id,
            owner_id: voyage.owner_id,
            title: voyage.title,
            description: voyage.description,
            published: voyage.published,
            granularity: granularity.as_str(),
            granularity_label: granularity_label(granularity),
            precision_m: granularity.half_cell_m(),
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
#[tracing::instrument(name = "voyage.create", skip_all, fields(voyage.id = tracing::field::Empty), err(Debug))]
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

    tracing::Span::current().record("voyage.id", voyage.id.to_string());

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
/// Lock the owned voyage before reading its effective title/publication state.
/// Validation and update share that lock, preventing independent patches from
/// publishing an empty title. The SQL update also checks ownership.
async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRequest>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let mut tx = state.pool.begin().await?;
    // Hold the voyage lock through validation and update: title and publication
    // are one invariant even when two requests patch different fields.
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(id)
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound.into());
    }
    let current = repo::find_by_id(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound)?;

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
    let voyage = repo::update(&mut *tx, id, user.user_id, &patch)
        .await?
        .ok_or(AppError::NotFound)?;

    let photos = repo::photos_for_voyage(&mut *tx, id).await?;
    tx.commit().await?;

    Ok(Json(VoyageResponse::build(voyage, photos)))
}

/// Deletes a voyage, owner only.
///
/// Deletes a voyage and its photo records (D11); shared files survive.
async fn destroy(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Rejection> {
    let mut tx = state.pool.begin().await?;
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(id)
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound.into());
    }
    let hashes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT content_hash FROM photos WHERE voyage_id=$1 ORDER BY content_hash",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    for hash in &hashes {
        photos_repo::lock_content(&mut tx, hash).await?;
    }
    repo::delete(&mut *tx, id, user.user_id).await?;
    for hash in &hashes {
        crate::photos::cleanup::schedule(&mut tx, hash).await?;
    }
    tx.commit().await?;
    for hash in hashes {
        if let Err(error) =
            crate::photos::cleanup::clean(&state.pool, &state.config.photos.storage_root, &hash)
                .await
        {
            tracing::warn!(%error, "voyage deletion committed; file cleanup will be retried");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Photos in a voyage
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PhotoIdsRequest {
    photo_ids: Vec<Uuid>,
}

/// Deletes a photo belonging to this voyage; never creates a library record.
async fn detach(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, photo_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Rejection> {
    let belongs: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM photos WHERE id=$1 AND voyage_id=$2 AND owner_id=$3)",
    )
    .bind(photo_id)
    .bind(id)
    .bind(user.user_id)
    .fetch_one(&state.pool)
    .await?;
    if !belongs {
        return Err(AppError::NotFound.into());
    }
    super::photos::destroy(State(state), user, Path(photo_id))
        .await
        .map_err(Rejection::App)
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
/// Published voyages allow anonymous reads; drafts require the owner (D6).
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
/// it would be easiest to leave out of the check, and it is the one the map
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
        precision_m: granularity.half_cell_m(),
        unplaced_count: unplaced.len(),
        pins,
        unplaced,
    }))
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
        let before = query
            .before
            .as_deref()
            .map(|value| repo::PublishedCursor::parse(value).ok_or_else(bad_cursor))
            .transpose()?;

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
                    repo::PublishedCursor::of(voyage).map(|cursor| cursor.encode())
                }
            })
        })
        .flatten();

    let voyages = summaries(&state.pool, voyages).await?;
    Ok(Json(ListResponse {
        voyages,
        next_before,
    }))
}

/// Add processed covers to either the public feed, owner list, or named profile.
pub(crate) async fn summaries(
    pool: &sqlx::PgPool,
    voyages: Vec<Voyage>,
) -> Result<Vec<VoyageSummary>, sqlx::Error> {
    let ids: Vec<_> = voyages.iter().map(|voyage| voyage.id).collect();
    let covers: std::collections::HashMap<_, _> = repo::covers_for_voyages(pool, &ids)
        .await?
        .into_iter()
        .map(|cover| (cover.voyage_id, cover))
        .collect();
    let voyages = voyages
        .into_iter()
        .map(|voyage| {
            let mut summary = VoyageSummary::from(voyage);
            if let Some(cover) = covers.get(&summary.id) {
                summary.cover_url = cover.photo_id.map(|id| variant_url(id, VariantKind::Thumb));
                summary.photos_pending = cover.pending;
            }
            summary
        })
        .collect();
    Ok(voyages)
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
/// Response details beyond [`AppError`]: a 422 carrying
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
}

impl Rejection {
    fn field(field: &'static str, message: &'static str) -> Self {
        Rejection::Field { field, message }
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
        }
    }
}

#[cfg(test)]
#[path = "voyages_tests.rs"]
mod tests;
