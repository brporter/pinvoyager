//! Photo endpoints.
//!
//! CONTRACT — `routes()` is fixed; handlers owned by the worker/serving agent.
//!
//! ```text
//! POST   /api/voyages/{id}/photos      upload directly into an owned voyage
//! GET    /api/voyages/{id}/photos      owner-only voyage photo metadata
//! GET    /api/photos/{id}              metadata
//! GET    /api/photos/{id}/{variant}    the image itself
//! DELETE /api/photos/{id}              owner only
//! ```
//!
//! The serving handler is where D7 is enforced, and it is why a static file
//! server is not acceptable here. It must set:
//!
//! * `Content-Type` determined by **us** — always `image/webp`, since every
//!   variant is one — never echoed from the upload
//! * `X-Content-Type-Options: nosniff`
//! * a restrictive `Content-Disposition`
//! * `ETag` / `Cache-Control: immutable` with a long max-age, which is safe
//!   precisely because the path is content-derived: the bytes at a given URL
//!   can never change
//!
//! The failure being designed against is a stored XSS: a user uploads a file
//! the browser sniffs as HTML and it executes on our origin against another
//! user's session.
//!
//! # Two things in here that exist only because of the dependency set
//!
//! Both are marked at their definitions, both are correct as written, and both
//! become deletable with a one-line `Cargo.toml` change that this agent was not
//! permitted to make:
//!
//! * [`Multipart`] is a hand-written streaming reader. axum's `multipart`
//!   feature is **not** enabled in this workspace, so `axum::extract::Multipart`
//!   does not exist here. Enabling it (`axum = { features = ["multipart"] }`)
//!   makes this whole section deletable.
//! * [`variant_body`] reads a variant into memory rather than streaming it out.
//!   Streaming a file into a response body requires naming either
//!   `futures_core::Stream` (for `Body::from_stream`) or `http_body::Frame` (to
//!   implement `HttpBody` by hand), and **neither type is nameable from this
//!   crate**: axum re-exports the `HttpBody` trait but not the frame type, and
//!   nothing else in the tree re-exports either. Adding
//!   `tokio-util = { version = "0.7", features = ["io"] }` reduces it to
//!   `Body::from_stream(ReaderStream::new(file))`.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

use crate::auth::{CurrentUser, MaybeUser};
use crate::blocking::run_blocking_infallible;
use crate::db::repo::photos::{self, NewPhoto, Photo};
use crate::db::repo::voyages as voyages_repo;
use crate::error::AppError;
use crate::photos::storage::{self, ContentHash};
use crate::photos::worker::{STATUS_PENDING, STATUS_READY, VariantJob};
use crate::photos::{ImageFormat, PhotoError, VariantKind, exif, validate};
use crate::state::AppState;

/// How many leading bytes are kept for the magic-byte sniff.
///
/// HEIC is why this is not four: it is an ISO-BMFF container whose `ftyp` box
/// brand sits several bytes in, so a window short enough for JPEG's two-byte
/// prefix would identify every iPhone photo as nothing at all.
const SNIFF_BYTES: usize = 32;

/// Default and maximum page size for voyage photo metadata.
const DEFAULT_PAGE: i64 = 50;
const MAX_PAGE: i64 = 100;

/// One year, `immutable`. Safe only because the bytes behind a variant URL are
/// content-derived and therefore cannot change.
const IMAGE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// All photo routes.
///
/// The variant segment is resolved through [`VariantKind::parse`] inside the
/// handler rather than interpolated into a path, which is what makes path
/// traversal a non-category here (T9): `../` in that position is simply not one
/// of the three names, and the path that is built comes from a hex hash and an
/// enum, never from a caller's string.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/voyages/{id}/photos", post(upload_to_voyage).get(list))
        .route("/api/photos/{id}", get(metadata).delete(destroy))
        .route("/api/photos/{id}/{variant}", get(image))
}

// ---------------------------------------------------------------------------
// Upload
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct UploadResponse {
    photos: Vec<PhotoResponse>,
}

/// Accepts one or many files (T1).
///
/// The body is **streamed**: each part goes to a temporary file through
/// `tokio::fs` as it arrives, so a 25 MB upload costs a 64 KiB buffer rather
/// than 25 MB of resident memory, and fifty of them do not multiply that.
///
/// Validation runs cheapest-first, and the order is the design (T2): the size
/// cap before the body is read, the magic bytes as soon as enough leading bytes
/// exist and before any decoder is handed the file, the dimensions from the
/// header before anything is fully decoded. The filename and the client's
/// `Content-Type` are consulted for nothing.
///
/// **A batch is not a transaction.** A file rejected part-way through fails the
/// request with the status that names the reason, and files accepted before it
/// stay in their voyage. They are photographs that passed every check, were
/// stored and were enqueued; discarding them would make a user re-upload good
/// files because of a bad one.
async fn owned_voyage(state: &AppState, user: &CurrentUser, id: Uuid) -> Result<(), Rejection> {
    let voyage = voyages_repo::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if voyage.owner_id != user.user_id {
        return Err(AppError::NotFound.into());
    }
    Ok(())
}

async fn upload_to_voyage(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(voyage_id): Path<Uuid>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadResponse>), Rejection> {
    owned_voyage(&state, &user, voyage_id).await?;
    upload(State(state), user, voyage_id, headers, body).await
}

async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    voyage_id: Uuid,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadResponse>), Rejection> {
    let limits = &state.config.photos;

    // T12, and first because it is the only check that costs nothing. A limiter
    // that fires half way through a body has already paid for the transfer.
    take_upload_token(user.user_id, limits.upload_rate_per_minute)?;

    // The declared length, before a byte of the body is read: a truthful client
    // gets its 413 immediately. A lying one — or a chunked body, which declares
    // no length at all — hits the same limit mid-stream in [`Multipart::fill`],
    // which is why both checks exist.
    if let Some(declared) = content_length(&headers)
        && declared > limits.max_upload_bytes
    {
        return Err(Rejection::too_large());
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let mut multipart = Multipart::new(content_type, body, limits.max_upload_bytes)?;

    // Temporaries live under the storage root rather than in the system temp
    // directory, so committing an upload is a rename within one filesystem. A
    // rename across filesystems is a copy, which would double the write cost of
    // every upload and stop being atomic.
    let temp_dir = limits.storage_root.join("tmp");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(PhotoError::Storage)?;

    let mut created = Vec::new();

    while let Some(part) = multipart.next_part().await? {
        if part.filename.is_none() {
            // Not a file part. Skipped rather than refused: the upload form may
            // grow ordinary fields, and this endpoint has no use for them yet.
            // `next_part` drains the body of a part the caller did not read.
            continue;
        }

        created.push(ingest(&state, user.user_id, voyage_id, &mut multipart, &temp_dir).await?);
    }

    if created.is_empty() {
        return Err(AppError::BadRequest("The request contained no files.".to_owned()).into());
    }

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse { photos: created }),
    ))
}

/// One file, from the wire to a row and a queued job.
///
/// Wraps [`ingest_part`] only so that the temporary file is removed on every
/// failure path. A `Drop` guard would be the tidier shape but cannot `await`,
/// and a synchronous unlink on a runtime worker thread is the thing D8 forbids.
async fn ingest(
    state: &AppState,
    owner_id: Uuid,
    voyage_id: Uuid,
    multipart: &mut Multipart,
    temp_dir: &FsPath,
) -> Result<PhotoResponse, Rejection> {
    // Content addressing means no caller-supplied string ever reaches the
    // filesystem — including here, where the name is a fresh uuid.
    let temp_path = temp_dir.join(format!("{}.part", Uuid::now_v7()));

    let result = ingest_part(state, owner_id, voyage_id, multipart, &temp_path).await;

    if result.is_err() {
        discard_failed_upload(&temp_path).await;
    }

    result
}

async fn discard_failed_upload(path: &FsPath) {
    // Best effort: a leaked temporary is a disk-space problem, and reporting
    // an upload that failed as though it had not is a correctness one.
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            let error_message = err.to_string();
            tracing::warn!(error = %error_message, "could not remove a rejected upload's temporary file");
        }
    }
}

async fn ingest_part(
    state: &AppState,
    owner_id: Uuid,
    voyage_id: Uuid,
    multipart: &mut Multipart,
    temp_path: &FsPath,
) -> Result<PhotoResponse, Rejection> {
    let limits = &state.config.photos;

    let mut file = tokio::fs::File::create(temp_path)
        .await
        .map_err(PhotoError::Storage)?;

    let mut header = Vec::with_capacity(SNIFF_BYTES);
    let mut format: Option<ImageFormat> = None;
    let mut byte_size: u64 = 0;

    while let Some(chunk) = multipart.next_chunk().await? {
        byte_size += chunk.len() as u64;
        if byte_size > limits.max_upload_bytes {
            return Err(Rejection::too_large());
        }

        if format.is_none() && header.len() < SNIFF_BYTES {
            let take = (SNIFF_BYTES - header.len()).min(chunk.len());
            header.extend_from_slice(&chunk[..take]);

            // Gate 2, and the stored-XSS defense. The moment enough bytes exist
            // to identify the format, an unrecognized one ends the upload —
            // before the rest of the body is written, and long before any
            // decoder is handed the file. A `.jpg` full of HTML lands here.
            if header.len() == SNIFF_BYTES {
                format = Some(sniff(&header)?);
            }
        }

        file.write_all(&chunk).await.map_err(PhotoError::Storage)?;
    }

    // Flushed and closed before anything reads it back: the probe and the hash
    // below open the same path, and an unflushed tail would make them disagree
    // with what was uploaded.
    file.flush().await.map_err(PhotoError::Storage)?;
    drop(file);

    // A file shorter than the sniff window never reached the check above. It is
    // still subject to it — and an empty part fails it, which is also the right
    // answer for a column that requires `byte_size > 0`.
    let format = match format {
        Some(format) => format,
        None => sniff(&header)?,
    };

    let max_megapixels = limits.max_megapixels;
    let probe_path = temp_path.to_path_buf();

    // D8: the dimension probe, the hash and the EXIF read are all CPU-bound or
    // blocking, and they run as **one** blocking task rather than three, per
    // the note in `crate::blocking` — `tokio::fs` is `spawn_blocking`
    // underneath, so a sequence of file operations is cheaper wrapped once.
    //
    // `run_blocking_infallible` with an inner `Result` rather than
    // `run_blocking`: the domain error has to survive the trip so that *this*
    // call site decides whether it is a 415 or a 422. The outer `?` is the
    // panic case, which for a decoder handed hostile bytes is a real one.
    let probed = run_blocking_infallible(move || {
        // Gate 3, before any full decode: a 50,000 × 50,000 PNG is a few
        // hundred kilobytes on disk and about 10 GB decoded, so only a pixel
        // count catches it — a byte-size limit never will.
        let (width, height) = validate::probe_dimensions(&probe_path, format)?;
        validate::check_pixel_budget(width, height, max_megapixels)?;

        // Only now, on bytes already known to be an image of a sane size.
        let hash = ContentHash::of_file(&probe_path)?;
        // Read from the original. Absent EXIF is the normal case, not an error.
        let exif = exif::extract(&probe_path);

        Ok::<_, PhotoError>((width, height, hash, exif))
    })
    .await?;

    let (width, height, hash, exif) = probed?;

    // Idempotent, and the dedup path: identical bytes already stored means the
    // temporary is discarded and one file backs both rows (T3).
    let mut tx = state.pool.begin().await?;
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(voyage_id)
            .bind(owner_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound.into());
    }
    photos::lock_content(&mut tx, hash.as_str()).await?;
    storage::commit_original(&limits.storage_root, temp_path, &hash).await?;
    let photo = photos::create(
        &mut *tx,
        &NewPhoto {
            voyage_id,
            owner_id,
            content_hash: hash.as_str().to_owned(),
            byte_size: i64::try_from(byte_size).unwrap_or(i64::MAX),
            width: i32::try_from(width).unwrap_or(i32::MAX),
            height: i32::try_from(height).unwrap_or(i32::MAX),
            original_format: format.as_str().to_owned(),
            captured_at: exif.captured_at,
            // Exact, as recorded. Granularity is applied at the serialization
            // boundary in phase 5; rounding here would discard the precision
            // irreversibly and make that decision unmakeable.
            latitude: exif.latitude,
            longitude: exif.longitude,
        },
    )
    .await?;

    tx.commit().await?;

    // Off the request path (T7). This awaits when the queue is full, which is
    // backpressure surfacing as a slower upload — much better than an unbounded
    // queue, and the reason it is not `try_send`.
    state
        .variants
        .enqueue(VariantJob { photo_id: photo.id })
        .await
        .map_err(|_| {
            // The worker is gone, which means shutdown. Reporting "created"
            // while nothing will ever derive the variants would be a lie; the
            // row stays `pending` and the next startup sweep picks it up.
            tracing::error!(photo_id = %photo.id, "variant queue is closed");
            AppError::ServiceUnavailable
        })?;

    Ok(PhotoResponse::from(photo))
}

/// The magic-byte gate. Never the extension, never the client's `Content-Type`.
fn sniff(header: &[u8]) -> Result<ImageFormat, Rejection> {
    validate::sniff_format(header).ok_or_else(Rejection::not_an_image)
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// One photo as the API describes it.
///
/// Coordinates are the **exact** stored values. That is correct here and only
/// here: every endpoint in this module is owner-only, and the owner is entitled
/// to their own precision. Granularity is a phase 5 concern applied at the
/// serialization boundary of the public surface — inventing a rounding rule in
/// this file would put a second implementation of it in the codebase, which is
/// exactly how such rules come to disagree.
#[derive(Debug, Serialize)]
struct PhotoResponse {
    id: Uuid,
    width: i32,
    height: i32,
    byte_size: i64,
    original_format: String,
    captured_at: Option<DateTime<Utc>>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    /// Whether the photo has coordinates at all. Absent GPS is ordinary —
    /// screenshots and scans have none — so this is a state, not a failure.
    placed: bool,
    /// `pending`, `ready` or `failed`. Readers must handle `pending`: a photo
    /// exists before its derived images do.
    variant_status: String,
    created_at: DateTime<Utc>,
}

impl From<Photo> for PhotoResponse {
    fn from(photo: Photo) -> Self {
        let latitude = photo.latitude.and_then(|value| value.to_f64());
        let longitude = photo.longitude.and_then(|value| value.to_f64());

        Self {
            id: photo.id,
            width: photo.width,
            height: photo.height,
            byte_size: photo.byte_size,
            original_format: photo.original_format,
            captured_at: photo.captured_at,
            placed: latitude.is_some() && longitude.is_some(),
            latitude,
            longitude,
            variant_status: photo.variant_status,
            created_at: photo.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    /// Cursor: the id of the oldest photo already seen. Ids are uuid v7, so
    /// ordering by id is chronological and the cursor needs no second column.
    before: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    photos: Vec<PhotoResponse>,
    /// What to pass as `before` for the next page, or null at the end.
    next_before: Option<Uuid>,
}

/// One voyage's owner-only photos, newest first.
///
/// Cursor rather than offset pagination: an upload landing between two requests
/// shifts every offset by one, which makes a page boundary skip or repeat a
/// photo — and on a voyage that is actively receiving uploads that is the
/// common case rather than the rare one.
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(voyage_id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListResponse>, Rejection> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);

    owned_voyage(&state, &user, voyage_id).await?;
    let photos =
        photos::list_for_voyage(&state.pool, user.user_id, voyage_id, query.before, limit).await?;

    // A full page implies there may be more; a short one is the end. Cheaper
    // than a count, and a count would be stale the moment it was taken.
    let next_before = if photos.len() as i64 == limit {
        photos.last().map(|photo| photo.id)
    } else {
        None
    };

    Ok(Json(ListResponse {
        photos: photos.into_iter().map(PhotoResponse::from).collect(),
        next_before,
    }))
}

/// One photo's metadata.
async fn metadata(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PhotoResponse>, Rejection> {
    Ok(Json(PhotoResponse::from(owned(&state, &user, id).await?)))
}

/// Loads a photo, or 404s.
///
/// A photo owned by somebody else is 404 rather than 403: a 403 confirms that
/// the id names something real, which an ownership check is not there to
/// reveal. Phase 5 relaxes this for photos attached to a voyage, which are
/// public by D6 — these are owner-only exact coordinates, and this is the
/// uploader's.
async fn owned(state: &AppState, user: &CurrentUser, id: Uuid) -> Result<Photo, Rejection> {
    let photo = photos::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;

    if photo.owner_id != user.user_id {
        return Err(AppError::NotFound.into());
    }

    Ok(photo)
}

/// Resolves a photo the caller is allowed to **look at**, which is a wider set
/// than the one they own.
///
/// Used only for serving image bytes. Every voyage is public (D6), so a
/// photograph attached to a published voyage is public too — and without this
/// the product is a public gallery with the pictures painted out: the voyage
/// JSON is world-readable while every `thumbnail_url` in it 404s for anyone but
/// the owner.
///
/// Metadata endpoints deliberately do **not** use this. They return exact
/// coordinates to an owner, and the granularity boundary lives in
/// `http::voyages`; widening those would route around it.
///
/// The ownership check comes first because it needs no second query, and a
/// draft never grants visibility — attaching a photo to an unpublished voyage
/// must not make it fetchable.
async fn viewable(
    state: &AppState,
    user: Option<&CurrentUser>,
    id: Uuid,
) -> Result<Photo, Rejection> {
    let photo = photos::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;

    if user.is_some_and(|user| user.user_id == photo.owner_id) {
        return Ok(photo);
    }

    if voyages_repo::photo_is_publicly_visible(&state.pool, id).await? {
        return Ok(photo);
    }

    // 404 rather than 403, consistently with the rest of the service: telling a
    // caller that a photo exists but is not theirs is an enumeration oracle.
    Err(AppError::NotFound.into())
}

// ---------------------------------------------------------------------------
// Serving — where D7 is enforced
// ---------------------------------------------------------------------------

/// Serves one variant (T9).
///
/// This handler is the entire reason files are not served from disk. Every
/// header below is one that a static file server would either take from the
/// upload or not set at all, and the combination is what makes an uploaded file
/// unable to execute as a document on this origin.
async fn image(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Path((id, variant)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Response, Rejection> {
    // A closed set of three names. This is the path-traversal answer: `../` is
    // not one of them, so it 404s before any path is built.
    let Some(kind) = VariantKind::parse(&variant) else {
        return Err(AppError::NotFound.into());
    };

    let photo = viewable(&state, user.as_ref(), id).await?;

    // A variant that does not exist *yet* is not a server error, and must not
    // be reported as one.
    availability(&photo.variant_status)?;

    let hash = ContentHash(photo.content_hash);
    let path = storage::variant_path(&state.config.photos.storage_root, &hash, kind);

    let (bytes, modified) = match variant_body(&path).await {
        Ok(found) => found,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // The row says ready and the file is gone: an operational problem,
            // not the caller's. 404 is still the honest answer to give them.
            tracing::warn!(photo_id = %id, variant = kind.as_str(), "variant file is missing");
            return Err(AppError::NotFound.into());
        }
        Err(err) => return Err(PhotoError::Storage(err).into()),
    };

    let etag = etag(hash.as_str(), kind);
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag));

    let image_headers = image_headers(&etag, bytes.len() as u64, modified);

    // A 304 carries the validators and no body — a body on a 304 is a protocol
    // error, and not re-sending the image is the entire point.
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Body::from(bytes).into_response()
    };
    response.headers_mut().extend(image_headers);

    Ok(response)
}

/// Reads a variant off disk.
///
/// **This buffers, and it should stream.** See the module docs: streaming a
/// file into a response body needs a nameable `Stream` or `Frame` type, and
/// this crate has neither. The buffering is bounded — a variant is a WebP whose
/// longest edge is at most 2560 px, so hundreds of kilobytes, never the 25 MB
/// original, which is never served at all (D6) — but the correct implementation
/// is `Body::from_stream(ReaderStream::new(file))` and it becomes available the
/// moment `tokio-util` with the `io` feature is a dependency.
///
/// `tokio::fs::read` is itself `spawn_blocking` underneath, so the read is off
/// the runtime worker threads either way (D8).
async fn variant_body(path: &FsPath) -> Result<(Vec<u8>, Option<SystemTime>), std::io::Error> {
    let bytes = tokio::fs::read(path).await?;
    let modified = tokio::fs::metadata(path)
        .await
        .ok()
        .and_then(|meta| meta.modified().ok());

    Ok((bytes, modified))
}

/// Whether a variant can be served at all, given the row's status.
///
/// Split out, and total over the string, because "the variants are not ready
/// yet" is the state most easily reached by accident: an upload returns before
/// its variants exist, so a client that follows the id straight to the image
/// hits it every time. Answering that with a 500 would make an ordinary race
/// look like a fault.
fn availability(status: &str) -> Result<(), Rejection> {
    match status {
        STATUS_READY => Ok(()),
        // 409 rather than 404: the photo exists, the caller may see it, and the
        // answer changes on its own. `Retry-After` says roughly when.
        STATUS_PENDING => Err(Rejection::pending()),
        // Failed, or a status this build does not know. The file will never
        // exist, so 404 is the truth and a retry is pointless.
        _ => Err(AppError::NotFound.into()),
    }
}

/// The entity tag for a variant.
///
/// Content-derived: the same bytes under the same variant always produce the
/// same tag, and two different originals cannot collide without a SHA-256
/// collision. That is also what makes `immutable` honest — the response at this
/// URL cannot change, so there is nothing for a year-long cache to get wrong.
///
/// It deliberately does not cover the encoder settings: changing `webp_quality`
/// does not change the tag. With a one-year `immutable` cache that is academic,
/// since no client would re-fetch in any case — but it is the reason a quality
/// change is a new storage generation rather than a silent re-encode.
fn etag(hash: &str, kind: VariantKind) -> String {
    format!("\"{hash}-{}\"", kind.as_str())
}

/// Every header on a served image, in one place, so that the set can be
/// asserted without a database, a file, or a request.
fn image_headers(etag: &str, length: u64, modified: Option<SystemTime>) -> HeaderMap {
    let mut headers = HeaderMap::new();

    // Determined by us. Every variant is a WebP because we encoded it as one;
    // nothing here is echoed from the upload, which is the whole point.
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/webp"));
    // Without this a browser may sniff the bytes and decide the response is
    // HTML — the stored-XSS path this design exists to close.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(IMAGE_CACHE_CONTROL),
    );

    // The filename is ours, derived from the tag, and never the uploader's
    // string — a caller-supplied filename here is where header injection and
    // double-extension tricks live. `inline` rather than `attachment` because
    // the product renders these in `<img>`, and the anti-sniffing guarantee
    // comes from the `Content-Type`/`nosniff` pair above rather than from
    // forcing a download.
    if let Ok(value) = HeaderValue::from_str(&format!(
        "inline; filename=\"{}.webp\"",
        etag.trim_matches('"')
    )) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }

    if let Ok(value) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, value);
    }

    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }

    if let Some(value) = modified
        .map(http_date)
        .and_then(|date| HeaderValue::from_str(&date).ok())
    {
        headers.insert(header::LAST_MODIFIED, value);
    }

    headers
}

/// An HTTP-date, which is RFC 1123 in GMT and nothing else.
///
/// Formatted by hand because `chrono`'s `to_rfc2822` writes `+0000` where the
/// HTTP grammar requires the literal `GMT`, and no date-formatting crate is a
/// dependency. `%a` and `%b` are English regardless of locale, which is what
/// the grammar wants.
fn http_date(time: SystemTime) -> String {
    DateTime::<Utc>::from(time)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Deletes a photo, owner only (T11).
///
/// Files are unlinked **only** when the row delete reports the content
/// unreferenced. Deduplication means two users can share one file, so an
/// unqualified unlink here removes another user's photograph — and the count
/// and the delete happen in one statement inside the repository, because a
/// concurrent upload of the same bytes can otherwise slip between them.
pub(super) async fn destroy(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut tx = state.pool.begin().await?;
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT voyage_id, content_hash FROM photos WHERE id=$1 AND owner_id=$2")
            .bind(id)
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((voyage_id, hash)) = row else {
        return Err(AppError::NotFound);
    };
    sqlx::query("SELECT id FROM voyages WHERE id=$1 FOR UPDATE")
        .bind(voyage_id)
        .execute(&mut *tx)
        .await?;
    photos::lock_content(&mut tx, &hash).await?;
    let Some((hash, orphaned)) =
        photos::delete_returning_orphaned(&mut *tx, id, user.user_id).await?
    else {
        return Err(AppError::NotFound);
    };

    if orphaned {
        // Best effort, deliberately. The row is already gone, so the caller's
        // intent has been carried out; failing the request now would report a
        // deletion that did happen as an error, and the retry would 404. Files
        // left behind are an operational problem with a log line attached, not
        // a correctness one.
        if let Err(err) =
            storage::remove_content(&state.config.photos.storage_root, &ContentHash(hash)).await
        {
            tracing::error!(photo_id = %id, error = %err, "could not remove unreferenced photo files");
        }
    }

    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Per-user upload rate limiting (T12)
// ---------------------------------------------------------------------------

/// Token buckets, keyed by user.
///
/// A process global rather than a field on [`AppState`], because `AppState` is
/// a fixed contract and this is its only consumer. Two consequences worth
/// stating rather than discovering: the budget resets when the process does,
/// and it is per-replica rather than per-cluster. Both are fine for a limit
/// whose job is to stop one account saturating one machine's disk and its
/// blocking pool; neither would be fine for a billing quota.
static UPLOAD_BUCKETS: LazyLock<Mutex<HashMap<Uuid, Bucket>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Entries idle for longer than this are dropped on the next sweep: a bucket
/// that has been full for two minutes is indistinguishable from a fresh one.
const BUCKET_IDLE: Duration = Duration::from_secs(120);

/// The map is only walked once it has grown enough to be worth walking, so the
/// common path stays one hash lookup.
const BUCKET_SWEEP_AT: usize = 4096;

struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Takes one token, or reports how many seconds until one exists.
///
/// A token bucket rather than a fixed window: a fixed window lets a user spend
/// a whole minute's budget in the last second of one window and again in the
/// first second of the next, which is twice the rate it claims to enforce, at
/// exactly the moment it matters.
fn take_upload_token(user_id: Uuid, per_minute: u32) -> Result<(), Rejection> {
    if per_minute == 0 {
        // Configured off. An honest 429 rather than the division by zero below.
        return Err(Rejection::rate_limited(60));
    }

    let capacity = f64::from(per_minute);
    let per_second = capacity / 60.0;
    let now = Instant::now();

    // A poisoned mutex means another caller panicked while holding it. What is
    // behind it is a rate-limit counter, not an invariant anything depends on,
    // so recovering beats propagating that panic into every later upload.
    let mut buckets = UPLOAD_BUCKETS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);

    if buckets.len() > BUCKET_SWEEP_AT {
        buckets.retain(|_, bucket| now.duration_since(bucket.updated) < BUCKET_IDLE);
    }

    let bucket = buckets.entry(user_id).or_insert(Bucket {
        tokens: capacity,
        updated: now,
    });

    let elapsed = now.duration_since(bucket.updated).as_secs_f64();
    bucket.tokens = (bucket.tokens + elapsed * per_second).min(capacity);
    bucket.updated = now;

    if bucket.tokens >= 1.0 {
        bucket.tokens -= 1.0;
        return Ok(());
    }

    // Rounded up, floored at a second: a `Retry-After: 0` invites an immediate
    // retry that is guaranteed to fail again.
    let wait = ((1.0 - bucket.tokens) / per_second).ceil().max(1.0);
    Err(Rejection::rate_limited(wait as u64))
}

// ---------------------------------------------------------------------------
// Streaming multipart reader
// ---------------------------------------------------------------------------
//
// Everything from here to the rejection type exists because this workspace does
// not enable axum's `multipart` feature, so `axum::extract::Multipart` is not
// available. It is a deliberately small subset of RFC 7578: enough to walk the
// parts of a `multipart/form-data` body while holding no more than one chunk
// plus a boundary's worth of lookahead in memory.
//
// Nothing it parses is trusted. The field name is used for nothing, the
// filename only to tell a file part from an ordinary field, and the part's own
// `Content-Type` is ignored entirely — the format comes from the magic bytes
// and from nowhere else.

/// Largest single header line, and the budget for a part's whole header block.
/// A part header is a handful of short lines; anything larger is a body that
/// has forgotten to end.
const MAX_HEADER_LINE: usize = 4 * 1024;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Phase {
    /// Before the first delimiter. Anything here is preamble, and is discarded.
    Preamble,
    /// A delimiter has just been consumed; a part's headers come next.
    Headers,
    /// Streaming a part's body.
    Body,
    /// The closing delimiter has been seen.
    Finished,
}

/// What a part's header block says, of the little that is used.
#[derive(Debug, Default)]
struct Part {
    /// Form field name. Recorded for logging and future use; nothing branches
    /// on it, so a request may name its file fields whatever it likes.
    #[allow(
        dead_code,
        reason = "captured for future use; nothing may branch on it"
    )]
    name: Option<String>,
    /// Present iff this is a file part. **Never** used as a path — storage is
    /// content-addressed precisely so that this string cannot reach the
    /// filesystem.
    filename: Option<String>,
}

struct Multipart {
    body: Body,
    /// `\r\n--{boundary}`. Every delimiter in a well-formed body is preceded by
    /// CRLF, and searching for the pair is what makes a boundary appearing
    /// *inside* image data harmless.
    delimiter: Vec<u8>,
    buf: Vec<u8>,
    /// The source is exhausted. Distinct from [`Phase::Finished`], which is
    /// about having seen the closing delimiter.
    eof: bool,
    phase: Phase,
    /// Bytes taken off the wire so far, and the cap they are checked against.
    /// Counted here rather than per part, so the multipart framing counts
    /// towards the limit too — which is the honest reading of "request size".
    read: u64,
    limit: u64,
}

impl Multipart {
    fn new(content_type: &str, body: Body, limit: u64) -> Result<Self, Rejection> {
        let boundary = boundary(content_type).ok_or_else(|| {
            AppError::BadRequest("Expected a multipart/form-data request body.".to_owned())
        })?;

        let mut delimiter = Vec::with_capacity(boundary.len() + 4);
        delimiter.extend_from_slice(b"\r\n--");
        delimiter.extend_from_slice(boundary.as_bytes());

        Ok(Self {
            body,
            delimiter,
            // Seeded with a synthetic CRLF so that the very first delimiter —
            // which sits at offset zero with nothing before it — matches the
            // same pattern as every later one. Cheaper than a special case that
            // would be exercised once per request and wrong the rest of the
            // time.
            buf: b"\r\n".to_vec(),
            eof: false,
            phase: Phase::Preamble,
            read: 0,
            limit,
        })
    }

    /// Advances to the next part, draining the current one if the caller did
    /// not read it to the end.
    async fn next_part(&mut self) -> Result<Option<Part>, Rejection> {
        loop {
            match self.phase {
                Phase::Finished => return Ok(None),
                Phase::Preamble => self.seek_delimiter().await?,
                Phase::Body => {
                    // The caller skipped this part — an ordinary form field —
                    // so its bytes still have to be walked to reach the
                    // delimiter that follows them.
                    while self.next_chunk().await?.is_some() {}
                }
                Phase::Headers => return self.read_part_headers().await,
            }
        }
    }

    /// The next slice of the current part's body, or `None` at its end.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Rejection> {
        if self.phase != Phase::Body {
            return Ok(None);
        }

        loop {
            if let Some(at) = find(&self.buf, &self.delimiter) {
                let chunk: Vec<u8> = self.buf.drain(..at).collect();
                self.buf.drain(..self.delimiter.len());
                self.phase = Phase::Headers;

                return Ok((!chunk.is_empty()).then_some(chunk));
            }

            if self.eof {
                // The body ended with no closing delimiter: a truncated upload,
                // or a client that disconnected. Either way the file is not
                // whole and must not be stored as though it were.
                return Err(malformed());
            }

            // Everything except a possible partial delimiter can be handed on.
            // Retaining `len - 1` bytes is what makes a delimiter split across
            // two frames still match.
            let keep = self.delimiter.len() - 1;
            if self.buf.len() > keep {
                let take = self.buf.len() - keep;
                return Ok(Some(self.buf.drain(..take).collect()));
            }

            self.fill().await?;
        }
    }

    /// Discards everything up to and including the next delimiter.
    async fn seek_delimiter(&mut self) -> Result<(), Rejection> {
        loop {
            if let Some(at) = find(&self.buf, &self.delimiter) {
                self.buf.drain(..at + self.delimiter.len());
                self.phase = Phase::Headers;
                return Ok(());
            }

            if self.eof {
                return Err(malformed());
            }

            // Preamble, which RFC 2046 says to ignore. Dropped rather than
            // accumulated, so a body that never contains a delimiter cannot
            // grow the buffer without bound.
            let keep = self.delimiter.len() - 1;
            if self.buf.len() > keep {
                let take = self.buf.len() - keep;
                self.buf.drain(..take);
            }

            self.fill().await?;
        }
    }

    /// Reads the header block that follows a delimiter.
    async fn read_part_headers(&mut self) -> Result<Option<Part>, Rejection> {
        // The rest of the delimiter's own line: `--` for the closing delimiter,
        // otherwise nothing but optional transport padding.
        let line = self.read_line().await?;
        if line.starts_with(b"--") {
            self.phase = Phase::Finished;
            return Ok(None);
        }
        if line.iter().any(|byte| !matches!(byte, b' ' | b'\t')) {
            return Err(malformed());
        }

        let mut part = Part::default();
        let mut budget = MAX_HEADER_LINE;

        loop {
            let line = self.read_line().await?;
            if line.is_empty() {
                break;
            }

            budget = budget.checked_sub(line.len()).ok_or_else(malformed)?;

            // Header names are ASCII; a line that is not UTF-8 is not one we
            // can act on, and skipping it is safe because nothing in here is
            // required to be present.
            let Ok(line) = std::str::from_utf8(&line) else {
                continue;
            };
            let Some((name, value)) = line.split_once(':') else {
                return Err(malformed());
            };

            if name.trim().eq_ignore_ascii_case("content-disposition") {
                part.name = parameter(value, "name");
                part.filename = parameter(value, "filename");
            }
            // The part's own `Content-Type` is read by nothing. It is an
            // attacker-controlled string that happens to look like metadata.
        }

        self.phase = Phase::Body;
        Ok(Some(part))
    }

    /// One CRLF-terminated line, without its terminator.
    async fn read_line(&mut self) -> Result<Vec<u8>, Rejection> {
        loop {
            if let Some(at) = find(&self.buf, b"\r\n") {
                if at > MAX_HEADER_LINE {
                    return Err(malformed());
                }
                let line: Vec<u8> = self.buf.drain(..at).collect();
                self.buf.drain(..2);
                return Ok(line);
            }

            if self.buf.len() > MAX_HEADER_LINE || self.eof {
                return Err(malformed());
            }

            self.fill().await?;
        }
    }

    /// Pulls one frame off the source, enforcing the request size cap.
    async fn fill(&mut self) -> Result<(), Rejection> {
        match next_frame(&mut self.body).await {
            Some(Ok(data)) => {
                self.read += data.len() as u64;
                if self.read > self.limit {
                    // T12: an honest 413 rather than a reset connection. Here
                    // as well as on `Content-Length`, because a chunked body
                    // declares no length and a dishonest one declares a wrong
                    // one.
                    return Err(Rejection::too_large());
                }
                self.buf.extend_from_slice(&data);
                Ok(())
            }
            Some(Err(err)) => {
                tracing::debug!(error = %err, "upload body could not be read");
                Err(AppError::BadRequest("The request body could not be read.".to_owned()).into())
            }
            None => {
                self.eof = true;
                Ok(())
            }
        }
    }
}

/// One data frame from a body.
///
/// Written against `HttpBody` directly because the `Stream` adapters live
/// behind crates this one does not depend on. Trailer frames carry no body
/// bytes, so they are skipped rather than ending the stream early.
async fn next_frame(body: &mut Body) -> Option<Result<Bytes, axum::Error>> {
    loop {
        let frame =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut *body).poll_frame(cx)).await?;

        match frame {
            Ok(frame) => match frame.into_data() {
                Ok(data) => return Some(Ok(data)),
                Err(_trailers) => continue,
            },
            Err(err) => return Some(Err(err)),
        }
    }
}

/// The boundary from a `multipart/form-data` content type, if it is one.
fn boundary(content_type: &str) -> Option<String> {
    let (mime, params) = content_type.split_once(';')?;
    if !mime.trim().eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }

    let boundary = parameter(params, "boundary")?;

    // RFC 2046: 1–70 characters. A zero-length boundary would match everywhere,
    // and an over-long one is not something any real client sends.
    (1..=70).contains(&boundary.len()).then_some(boundary)
}

/// A `name=value` or `name="value"` parameter out of a header's parameter list.
fn parameter(params: &str, wanted: &str) -> Option<String> {
    params.split(';').find_map(|param| {
        let (name, value) = param.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case(wanted) {
            return None;
        }

        let value = value.trim();
        // Quoted form. The unescaping is deliberately minimal: the only thing
        // read off these values is "is there a filename at all", and a cleverer
        // unescape would be a parser nobody reads.
        Some(
            value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or(value)
                .to_owned(),
        )
    })
}

/// First index of `needle` in `haystack`.
///
/// Scans for the first byte and only then compares, rather than
/// `windows().position`, which compares the whole needle at every offset — on a
/// 25 MB body with a 40-byte boundary that is a gigabyte of comparisons per
/// request.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    let last = haystack.len() - needle.len();
    let mut from = 0;

    while from <= last {
        let offset = haystack[from..=last]
            .iter()
            .position(|byte| *byte == needle[0])?;
        let at = from + offset;

        if &haystack[at..at + needle.len()] == needle {
            return Some(at);
        }
        from = at + 1;
    }

    None
}

fn malformed() -> Rejection {
    AppError::BadRequest("The multipart request body is malformed.".to_owned()).into()
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

/// A failure from one of these handlers.
///
/// It exists because [`AppError`]'s variants are a fixed contract and four of
/// the statuses this phase's design calls for — 413, 415, 422 and 429 — have no
/// variant to carry them. The response body has the same shape as `AppError`'s,
/// so a client sees one error format across the whole API regardless of which
/// type produced it.
#[derive(Debug)]
enum Rejection {
    App(AppError),
    Explicit {
        status: StatusCode,
        code: &'static str,
        message: &'static str,
        /// Seconds. Set on the two statuses that are answered by waiting.
        retry_after: Option<u64>,
    },
}

impl Rejection {
    /// T12, and the design's first rejection branch: an honest status, not a
    /// reset connection.
    fn too_large() -> Self {
        Rejection::Explicit {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "too_large",
            message: "That upload is larger than the limit.",
            retry_after: None,
        }
    }

    /// The magic-byte gate. A `.jpg` whose bytes are HTML lands here, and this
    /// is the stored-XSS defense rather than a formatting nicety.
    fn not_an_image() -> Self {
        Rejection::Explicit {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            code: "not_an_image",
            message: "That file is not an image we can read.",
            retry_after: None,
        }
    }

    /// The decompression-bomb guard, and anything else that is an image but not
    /// one this service can process.
    fn unprocessable(message: &'static str) -> Self {
        Rejection::Explicit {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "unprocessable_image",
            message,
            retry_after: None,
        }
    }

    fn rate_limited(retry_after: u64) -> Self {
        Rejection::Explicit {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited",
            message: "You are uploading faster than we can accept. Try again shortly.",
            retry_after: Some(retry_after),
        }
    }

    /// The variants for this photo do not exist yet. Not a 500, and not a
    /// permanent 404 — the answer changes on its own.
    fn pending() -> Self {
        Rejection::Explicit {
            status: StatusCode::CONFLICT,
            code: "variants_pending",
            message: "This photo is still being processed.",
            retry_after: Some(2),
        }
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

/// The mapping the design's rejection branches imply, made once, here — the
/// call site is where the knowledge lives about whether a failed decode is the
/// caller's fault or ours.
impl From<PhotoError> for Rejection {
    fn from(err: PhotoError) -> Self {
        match err {
            PhotoError::TooLarge => Rejection::too_large(),
            PhotoError::NotAnImage => Rejection::not_an_image(),
            PhotoError::TooManyPixels => {
                Rejection::unprocessable("That image has more pixels than we can process.")
            }
            PhotoError::Undecodable(ref reason) => {
                tracing::debug!(reason, "upload rejected as undecodable");
                Rejection::unprocessable("That image could not be read.")
            }
            // Ours, not the caller's: the bytes were fine and something on this
            // side failed to handle them.
            PhotoError::Unencodable(_) | PhotoError::Storage(_) => {
                Rejection::App(AppError::Internal(Box::new(err)))
            }
        }
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            // `AppError` logs and renders itself; there is nothing to add.
            Rejection::App(err) => err.into_response(),
            Rejection::Explicit {
                status,
                code,
                message,
                retry_after,
            } => {
                // A 4xx is the client's mistake, not ours — logged at debug for
                // the same reason `AppError` does it.
                tracing::debug!(status = status.as_u16(), code, "request rejected");

                let mut response =
                    (status, Json(json!({ "code": code, "message": message }))).into_response();

                if let Some(seconds) = retry_after
                    && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
                {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }

                response
            }
        }
    }
}

#[cfg(test)]
#[path = "photos_tests.rs"]
mod tests;
