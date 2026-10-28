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
//! Public detail, map, and discovery coordinates pass through `place_pair`,
//! which applies the voyage's granularity and returns a
//! [`crate::voyages::Coordinate`]. Summaries contain no coordinates.
//! Owner-only photo metadata in `http::photos` intentionally returns exact
//! coordinates. Repository rows do not implement `Serialize`; new response
//! types still need review to ensure they do not expose raw coordinate fields.
//!
//! Draft reads and owner-only mutations return 404 for foreign voyages.
//! Validation messages name fields without echoing coordinates.
//! `stored_granularity` uses the coarsest supported precision for an unknown
//! stored value, so an older build does not interpret it as exact.

use std::collections::HashSet;
mod discovery;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
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

/// Maximum title and description lengths, measured in Unicode characters.
const MAX_TITLE_LEN: usize = 200;
const MAX_DESCRIPTION_LEN: usize = 8_000;

/// Maximum number of photo IDs accepted in one reorder request.
const MAX_PHOTO_IDS: usize = 500;

// Fixed validation messages avoid including coordinates in error responses.
const TITLE_REQUIRED: &str = "A title is required before publishing.";
const TITLE_TOO_LONG: &str = "That title is longer than we can store.";
const DESCRIPTION_TOO_LONG: &str = "That description is longer than we can store.";
const UNKNOWN_GRANULARITY: &str = "That is not a location precision we recognize.";
const NO_PHOTOS_NAMED: &str = "Name at least one photograph.";
const TOO_MANY_PHOTOS: &str = "That is more photographs than one request can carry.";
const BAD_CURSOR: &str = "That page cursor is not one we issued.";
const MINE_REQUIRES_A_SESSION: &str = "Listing your own voyages requires signing in.";

/// Registers voyage CRUD, photo ordering/deletion, and public discovery routes.
/// The literal `photos/order` segment takes precedence over `photos/{photo}`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/voyages", post(create_request).get(list))
        .route(
            "/api/voyages/{id}",
            get(read).patch(update_request).delete(destroy_request),
        )
        .route("/api/voyages/{id}/photos/order", patch(reorder_request))
        .route(
            "/api/voyages/{id}/photo-order",
            axum::routing::put(complete_order),
        )
        .route("/api/voyages/{id}/photos/{photo}", delete(detach_request))
        .route("/api/voyages/{id}/map", get(map))
        .route("/api/map", get(discovery::discover_map))
}

// ---------------------------------------------------------------------------
// The boundary itself
// ---------------------------------------------------------------------------

/// Rounds a complete coordinate pair. Missing either axis leaves the photo
/// unplaced. Discovery already has complete pairs and calls `place_pair` directly.
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

/// Applies the selected precision for public voyage and discovery responses.
fn place_pair(granularity: Granularity, latitude: f64, longitude: f64) -> Coordinate {
    granularity.apply(latitude, longitude)
}

/// Converts a stored string to a granularity.
///
/// If the string is unrecognized, logs an error and returns `Granularity::Km50`,
/// the coarsest supported precision, to avoid exposing exact coordinates.
/// This function does not return an error.
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

/// Returns the display label for the selected precision: `Exact`, `~1 km`,
/// `~10 km`, or `~50 km`.
fn granularity_label(granularity: Granularity) -> &'static str {
    match granularity {
        Granularity::Exact => "Exact",
        Granularity::Km1 => "~1 km",
        Granularity::Km10 => "~10 km",
        Granularity::Km50 => "~50 km",
    }
}

/// Builds a variant URL from the photo ID and a supported variant name.
fn variant_url(photo_id: Uuid, kind: VariantKind) -> String {
    format!("/api/photos/{photo_id}/{}", kind.as_str())
}

/// Counts distinct coordinate bit patterns after rounding. Photos in the same
/// rounding cell count as one place, without revealing distinct exact positions.
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

/// Voyage detail with photo coordinates rounded to its selected granularity.
#[derive(Debug, Serialize)]
struct VoyageResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    id: Uuid,
    owner_id: Uuid,
    title: String,
    description: Option<String>,
    published: bool,
    /// Stored spelling — `exact`, `km1`, `km10`, `km50`. The value a PATCH
    /// sends back.
    granularity: &'static str,
    /// Display label for the selected location precision.
    granularity_label: &'static str,
    /// Nominal per-axis half-width of the rounding cell in meters; `null` for exact.
    precision_m: Option<u32>,
    created_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
    photo_count: usize,
    /// Photographs that have coordinates, and how many distinct points they
    /// draw once rounded. "48 photos · 12 places" comes from these two.
    placed_count: usize,
    place_count: usize,
    /// Number of photos without coordinates. These photos also appear in `photos`.
    unplaced_count: usize,
    photos: Vec<VoyagePhotoResponse>,
}

/// A voyage photo. Photos without coordinates remain in the response with
/// `coordinate: null` and `placed: false`.
#[derive(Debug, Serialize)]
struct VoyagePhotoResponse {
    id: Uuid,
    position: i32,
    captured_at: Option<DateTime<Utc>>,
    width: i32,
    height: i32,
    /// `pending`, `ready`, or `failed`. Variant URLs may not be available
    /// until processing finishes.
    variant_status: String,
    thumbnail_url: String,
    display_url: String,
    full_url: String,
    /// Coordinates at the selected precision, or `null` for an unplaced photo.
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

/// Map response with rounded pins and a separate list of unplaced photos.
#[derive(Debug, Serialize)]
struct MapResponse {
    voyage_id: Uuid,
    granularity: &'static str,
    granularity_label: &'static str,
    /// Repeated here as well as on every pin because the map needs it to size
    /// its areas even when the voyage has no pins at all.
    precision_m: Option<u32>,
    pins: Vec<MapPin>,
    /// Number of voyage photos without coordinates.
    unplaced_count: usize,
    /// Unplaced photos available for viewing even though they cannot appear as pins.
    unplaced: Vec<MapUnplaced>,
}

/// A placed photo represented by a rounded coordinate, ID, and thumbnail URL.
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
    /// Always `false`, including when clients combine this list with placed photos.
    placed: bool,
}

impl VoyageResponse {
    /// Builds voyage detail from repository rows and rounds photo coordinates.
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
            revision: None,
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

#[derive(Debug, Deserialize, Serialize)]
struct CreateRequest {
    /// Optional draft title. Publishing requires a nonblank title.
    title: Option<String>,
    description: Option<String>,
    /// Stored spelling. Absent defaults to `km1` in this handler.
    granularity: Option<String>,
}

/// Creates an unpublished voyage. Publication requires a separate PATCH request.
#[tracing::instrument(name = "voyage.create", skip_all, fields(voyage.id = tracing::field::Empty), err(Debug))]
async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<VoyageResponse>), Rejection> {
    let mut tx = state.pool.begin().await?;
    let response = create_in(&mut tx, user.user_id, body).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn create_in(
    connection: &mut sqlx::PgConnection,
    owner: Uuid,
    body: CreateRequest,
) -> Result<VoyageResponse, Rejection> {
    let title = clean_title(body.title.as_deref())?;
    let description = clean_description(body.description.as_deref())?;
    let granularity = body
        .granularity
        .as_deref()
        .map(parse_granularity)
        .transpose()?
        .unwrap_or(Granularity::Km1);
    crate::db::repo::users::require_active(connection, owner).await?;
    let voyage = repo::create(
        &mut *connection,
        &NewVoyage {
            owner_id: owner,
            title,
            description,
            granularity: granularity.as_str().to_owned(),
        },
    )
    .await?;
    let revision = super::mutations::revision(connection, voyage.id, None).await?;
    let mut response = VoyageResponse::build(voyage, Vec::new());
    response.revision = Some(revision.to_string());
    Ok(response)
}

async fn create_request(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<CreateRequest>,
) -> Result<Response, Rejection> {
    if !super::mutations::conditional(&headers) {
        return Ok(create(State(state), user, Json(body))
            .await?
            .into_response());
    }
    let payload = json!(body);
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "POST",
            path: "/api/voyages".to_owned(),
            payload,
            requires_revision: false,
        },
        async move |connection, _| {
            let response = create_in(connection, user.user_id, body).await?;
            let revision = super::mutations::revision(connection, response.id, None).await?;
            Ok(super::mutations::Outcome::json(
                StatusCode::CREATED,
                response.id,
                response.id,
                "voyage",
                revision,
                response,
            )?)
        },
    )
    .await
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct UpdateRequest {
    title: Option<String>,
    /// Three states, not two. Absent leaves the description alone; `null`
    /// clears it; a string sets it. A plain `Option<String>` cannot tell the
    /// first two apart, which would make "remove my description" impossible to
    /// express.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
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
    let response = update_in(&mut tx, id, user.user_id, body, None).await?;
    tx.commit().await?;
    Ok(Json(response))
}

async fn update_in(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
    owner: Uuid,
    body: UpdateRequest,
    expected: Option<i64>,
) -> Result<VoyageResponse, Rejection> {
    // Hold the voyage lock through validation and update: title and publication
    // are one invariant even when two requests patch different fields.
    lock_owned(connection, id, owner).await?;
    super::mutations::revision(connection, id, expected).await?;
    let current = repo::find_by_id(&mut *connection, id)
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

    // Validate the resulting state so publishing without a title and clearing
    // the title of a published voyage both fail.
    let effective_title = title.as_deref().unwrap_or(&current.title);
    let effective_published = body.published.unwrap_or(current.published);
    publishable(effective_title, effective_published)?;
    if effective_published {
        crate::voyages::publication::ensure_eligible(connection, id).await?;
    }

    let patch = VoyagePatch {
        title,
        description,
        granularity: granularity.map(|value| value.as_str().to_owned()),
        published: body.published,
    };

    // The lock above prevents concurrent deletion. Retain the repository's
    // ownership predicate and handle a missing row as 404.
    let voyage = repo::update(&mut *connection, id, owner, &patch)
        .await?
        .ok_or(AppError::NotFound)?;

    let photos = repo::photos_for_voyage(&mut *connection, id).await?;
    let revision = super::mutations::revision(connection, id, None).await?;
    let mut response = VoyageResponse::build(voyage, photos);
    response.revision = Some(revision.to_string());
    Ok(response)
}

async fn update_request(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateRequest>,
) -> Result<Response, Rejection> {
    if !super::mutations::conditional(&headers) {
        return Ok(update(State(state), user, Path(id), Json(body))
            .await?
            .into_response());
    }
    let payload = json!(body);
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "PATCH",
            path: format!("/api/voyages/{id}"),
            payload,
            requires_revision: true,
        },
        async move |connection, expected| {
            let response = update_in(connection, id, user.user_id, body, expected).await?;
            let revision = super::mutations::revision(connection, id, None).await?;
            Ok(super::mutations::Outcome::json(
                StatusCode::OK,
                id,
                id,
                "voyage",
                revision,
                response,
            )?)
        },
    )
    .await
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
    destroy_in(&mut tx, id, user.user_id, None).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn destroy_in(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
    owner: Uuid,
    expected: Option<i64>,
) -> Result<i64, Rejection> {
    lock_owned(connection, id, owner).await?;
    let revision = super::mutations::revision(connection, id, expected).await?;
    let hashes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT content_hash FROM photos WHERE voyage_id=$1 ORDER BY content_hash",
    )
    .bind(id)
    .fetch_all(&mut *connection)
    .await?;
    for hash in &hashes {
        photos_repo::lock_content(connection, hash).await?;
    }
    repo::delete(&mut *connection, id, owner).await?;
    for hash in &hashes {
        crate::photos::cleanup::schedule(connection, hash).await?;
    }
    Ok(revision + 1)
}

async fn destroy_request(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, Rejection> {
    if !super::mutations::conditional(&headers) {
        return Ok(destroy(State(state), user, Path(id)).await?.into_response());
    }
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "DELETE",
            path: format!("/api/voyages/{id}"),
            payload: json!(null),
            requires_revision: true,
        },
        async move |connection, expected| {
            let revision = destroy_in(connection, id, user.user_id, expected).await?;
            Ok(super::mutations::Outcome::deleted(
                id, id, "voyage", revision,
            ))
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Photos in a voyage
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
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

async fn detach_request(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, photo)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Response, Rejection> {
    if !super::mutations::conditional(&headers) {
        return Ok(detach(State(state), user, Path((id, photo)))
            .await?
            .into_response());
    }
    super::photos::destroy_with_headers(
        &state,
        user,
        photo,
        Some(id),
        &headers,
        format!("/api/voyages/{id}/photos/{photo}"),
    )
    .await
    .map_err(Rejection::App)
}

/// Sets an explicit order (T4).
///
/// Listed photos come first in request order. The repository ignores unknown
/// IDs and appends omitted photos in their previous relative order.
async fn reorder(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PhotoIdsRequest>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let mut tx = state.pool.begin().await?;
    let response = reorder_in(&mut tx, id, user.user_id, body, None, false).await?;
    tx.commit().await?;
    Ok(Json(response))
}

async fn reorder_in(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
    owner: Uuid,
    body: PhotoIdsRequest,
    expected: Option<i64>,
    complete: bool,
) -> Result<VoyageResponse, Rejection> {
    lock_owned(connection, id, owner).await?;
    super::mutations::revision(connection, id, expected).await?;
    let photo_ids = if complete {
        let actual: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM photos WHERE voyage_id=$1")
            .bind(id)
            .fetch_all(&mut *connection)
            .await?;
        if body.photo_ids.len() > 10_000
            || body.photo_ids.len() != actual.len()
            || body.photo_ids.iter().collect::<HashSet<_>>().len() != body.photo_ids.len()
            || body.photo_ids.iter().copied().collect::<HashSet<_>>()
                != actual.into_iter().collect()
        {
            return Err(AppError::BadRequest(
                "The order must include every photo exactly once.".to_owned(),
            )
            .into());
        }
        body.photo_ids
    } else {
        clean_photo_ids(body.photo_ids)?
    };
    let voyage = repo::find_by_id(&mut *connection, id)
        .await?
        .ok_or(AppError::NotFound)?;
    repo::reorder(&mut *connection, id, owner, &photo_ids).await?;
    let photos = repo::photos_for_voyage(&mut *connection, id).await?;
    let revision = super::mutations::revision(connection, id, None).await?;
    let mut response = VoyageResponse::build(voyage, photos);
    response.revision = Some(revision.to_string());
    Ok(response)
}

async fn reorder_request(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<PhotoIdsRequest>,
) -> Result<Response, Rejection> {
    if !super::mutations::conditional(&headers) {
        return Ok(reorder(State(state), user, Path(id), Json(body))
            .await?
            .into_response());
    }
    reorder_operation(&state, user.user_id, id, &headers, body, false).await
}

async fn complete_order(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<PhotoIdsRequest>,
) -> Result<Response, Rejection> {
    reorder_operation(&state, user.user_id, id, &headers, body, true).await
}

async fn reorder_operation(
    state: &AppState,
    owner: Uuid,
    id: Uuid,
    headers: &HeaderMap,
    body: PhotoIdsRequest,
    complete: bool,
) -> Result<Response, Rejection> {
    let payload = json!(body);
    let path = if complete {
        format!("/api/voyages/{id}/photo-order")
    } else {
        format!("/api/voyages/{id}/photos/order")
    };
    super::mutations::run(
        state,
        owner,
        headers,
        super::mutations::Request {
            method: if complete { "PUT" } else { "PATCH" },
            path,
            payload,
            requires_revision: true,
        },
        async move |connection, expected| {
            let response = reorder_in(connection, id, owner, body, expected, complete).await?;
            let revision = super::mutations::revision(connection, id, None).await?;
            Ok(super::mutations::Outcome::json(
                StatusCode::OK,
                id,
                id,
                "voyage",
                revision,
                response,
            )?)
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// Returns voyage detail with coordinates at the selected precision.
///
/// Published voyages allow anonymous reads; drafts require the owner. Loads
/// the voyage and its photos in two queries, regardless of photo count.
async fn read(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Path(id): Path<Uuid>,
) -> Result<Json<VoyageResponse>, Rejection> {
    let mut tx = state.pool.begin().await?;
    let found: Option<Uuid> = sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 FOR SHARE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    found.ok_or(AppError::NotFound)?;
    let voyage = repo::find_by_id(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let owner = user
        .as_ref()
        .is_some_and(|user| user.user_id == voyage.owner_id);
    if !super::moderation::can_view(&mut *tx, user.as_ref().map(|user| user.user_id), id).await? {
        return Err(AppError::NotFound.into());
    }
    let photos = repo::photos_for_voyage(&mut *tx, id).await?;
    let revision = if owner {
        Some(
            super::mutations::revision(&mut tx, id, None)
                .await?
                .to_string(),
        )
    } else {
        None
    };
    tx.commit().await?;
    let mut response = VoyageResponse::build(voyage, photos);
    response.revision = revision;
    Ok(Json(response))
}

/// Returns map data with the same visibility and precision rules as voyage detail.
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
    /// Opaque cursor: publication timestamp plus ID for the public feed,
    /// or a voyage ID for the owner list.
    before: Option<String>,
    limit: Option<i64>,
}

/// Lists public voyages or the signed-in owner's voyages using cursor pagination.
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

        repo::list_published_visible(
            &state.pool,
            before,
            limit,
            user.as_ref().map(|user| user.user_id),
        )
        .await?
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

/// Loads a visible voyage. Missing voyages and drafts requested by non-owners
/// both return 404.
async fn visible(
    state: &AppState,
    user: Option<&CurrentUser>,
    id: Uuid,
) -> Result<Voyage, Rejection> {
    let voyage = repo::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let readable =
        super::moderation::can_view(&state.pool, user.map(|user| user.user_id), id).await?;

    if !readable {
        return Err(AppError::NotFound.into());
    }

    Ok(voyage)
}

/// Checks suspension and ownership while holding account then voyage locks.
/// The locks remain held through the caller's transaction, so suspension and
/// account deletion cannot finish before this mutation commits or rolls back.
/// Suspended or deleted accounts return 403; missing or foreign voyages return 404.
async fn lock_owned(tx: &mut sqlx::PgConnection, id: Uuid, owner: Uuid) -> Result<(), Rejection> {
    crate::db::repo::users::require_active(tx, owner).await?;
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(id)
            .bind(owner)
            .fetch_optional(tx)
            .await?;
    found.ok_or(AppError::NotFound)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Rejects publication with a blank title using a field-level validation error.
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

    // Count Unicode characters so multibyte titles use the same length limit.
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

/// Removes duplicate photo IDs while preserving request order. Returns a
/// validation error for an empty list or an input longer than `MAX_PHOTO_IDS`.
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

/// Handler errors: shared application errors, field-specific 422 validation
/// errors, and 403 responses. Validation and forbidden messages are static strings.
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
