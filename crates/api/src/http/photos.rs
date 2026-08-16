//! Photo endpoints.
//!
//! CONTRACT — `routes()` is fixed; handlers owned by the worker/serving agent.
//!
//! ```text
//! POST   /api/photos                   multipart upload, streamed
//! GET    /api/photos                   the caller's library
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

use crate::auth::CurrentUser;
use crate::blocking::run_blocking_infallible;
use crate::db::repo::photos::{self, NewPhoto, Photo};
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

/// Default and maximum page size for the library listing.
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
        .route("/api/photos", post(upload).get(list))
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
/// stay in the library. They are photographs that passed every check, were
/// stored and were enqueued; discarding them would make a user re-upload good
/// files because of a bad one.
async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
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

        created.push(ingest(&state, user.user_id, &mut multipart, &temp_dir).await?);
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
    multipart: &mut Multipart,
    temp_dir: &FsPath,
) -> Result<PhotoResponse, Rejection> {
    // Content addressing means no caller-supplied string ever reaches the
    // filesystem — including here, where the name is a fresh uuid.
    let temp_path = temp_dir.join(format!("{}.part", Uuid::now_v7()));

    let result = ingest_part(state, owner_id, multipart, &temp_path).await;

    if result.is_err() {
        // Best effort: a leaked temporary is a disk-space problem, and reporting
        // an upload that failed as though it had not is a correctness one.
        match tokio::fs::remove_file(&temp_path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::warn!(error = %err, "could not remove a rejected upload's temporary file");
            }
        }
    }

    result
}

async fn ingest_part(
    state: &AppState,
    owner_id: Uuid,
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

            // Gate 2, and the stored-XSS defence. The moment enough bytes exist
            // to identify the format, an unrecognised one ends the upload —
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
    storage::commit_original(&limits.storage_root, temp_path, &hash).await?;

    let photo = photos::create(
        &state.pool,
        &NewPhoto {
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

/// The caller's library, newest first.
///
/// Cursor rather than offset pagination: an upload landing between two requests
/// shifts every offset by one, which makes a page boundary skip or repeat a
/// photo — and on a library that is actively being uploaded to that is the
/// common case rather than the rare one.
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListResponse>, Rejection> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);

    let photos = photos::list_for_owner(&state.pool, user.user_id, query.before, limit).await?;

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
/// public by D6 — this is the pre-voyage library, and the library is the
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
    user: CurrentUser,
    Path((id, variant)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Response, Rejection> {
    // A closed set of three names. This is the path-traversal answer: `../` is
    // not one of them, so it 404s before any path is built.
    let Some(kind) = VariantKind::parse(&variant) else {
        return Err(AppError::NotFound.into());
    };

    let photo = owned(&state, &user, id).await?;

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
async fn destroy(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Rejection> {
    // Not owned and not found are the same answer, produced by the same
    // statement: the delete is qualified by owner.
    let Some((hash, orphaned)) =
        photos::delete_returning_orphaned(&state.pool, id, user.user_id).await?
    else {
        return Err(AppError::NotFound.into());
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

/// Where a [`Multipart`]'s bytes come from.
///
/// The test variant exists because `Body::from(Vec<u8>)` delivers the entire
/// body as a single frame, and every interesting bug in a streaming parser is
/// about a delimiter or a header landing across a frame boundary. A test that
/// only ever sees one frame cannot find them.
enum Source {
    Body(Body),
    #[cfg(test)]
    Frames(std::collections::VecDeque<Bytes>),
}

impl Source {
    async fn next(&mut self) -> Option<Result<Bytes, axum::Error>> {
        match self {
            Source::Body(body) => next_frame(body).await,
            #[cfg(test)]
            Source::Frames(frames) => frames.pop_front().map(Ok),
        }
    }
}

struct Multipart {
    source: Source,
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
        Self::from_source(content_type, Source::Body(body), limit)
    }

    fn from_source(content_type: &str, source: Source, limit: u64) -> Result<Self, Rejection> {
        let boundary = boundary(content_type).ok_or_else(|| {
            AppError::BadRequest("Expected a multipart/form-data request body.".to_owned())
        })?;

        let mut delimiter = Vec::with_capacity(boundary.len() + 4);
        delimiter.extend_from_slice(b"\r\n--");
        delimiter.extend_from_slice(boundary.as_bytes());

        Ok(Self {
            source,
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
        match self.source.next().await {
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
    /// is the stored-XSS defence rather than a formatting nicety.
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
mod tests {
    use std::sync::Arc;

    use axum::http::Request;
    use secrecy::SecretString;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt as _;

    use super::*;
    use crate::config::{Config, DatabaseConfig, PhotosConfig, ServerConfig, TelemetryConfig};
    use crate::photos::worker::STATUS_FAILED;

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

    async fn status_of(method: &str, uri: &str) -> StatusCode {
        crate::http::router(test_state())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible")
            .status()
    }

    /// The routes exist and every one of them requires a session. 401 rather
    /// than 404 is also what proves they are merged into the router at all — an
    /// unwired route would 404 here, with no database involved either way.
    #[tokio::test]
    async fn every_photo_route_requires_a_session() {
        let id = Uuid::now_v7();

        for (method, uri) in [
            ("GET", "/api/photos".to_owned()),
            ("POST", "/api/photos".to_owned()),
            ("GET", format!("/api/photos/{id}")),
            ("DELETE", format!("/api/photos/{id}")),
            ("GET", format!("/api/photos/{id}/thumb")),
        ] {
            assert_eq!(
                status_of(method, &uri).await,
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
    }

    fn header_of(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// T9's header set, asserted without a database and without a file — which
    /// is what makes it a test that will still be run in a year.
    #[test]
    fn a_served_image_carries_the_security_headers() {
        let tag = etag(&"a".repeat(64), VariantKind::Display);
        let headers = image_headers(&tag, 4096, None);

        assert_eq!(
            header_of(&headers, header::CONTENT_TYPE),
            Some("image/webp"),
            "the type is ours, never the upload's"
        );
        assert_eq!(
            header_of(&headers, header::X_CONTENT_TYPE_OPTIONS),
            Some("nosniff")
        );
        assert_eq!(
            header_of(&headers, header::CACHE_CONTROL),
            Some(IMAGE_CACHE_CONTROL)
        );
        assert_eq!(header_of(&headers, header::ETAG), Some(tag.as_str()));
        assert_eq!(header_of(&headers, header::CONTENT_LENGTH), Some("4096"));

        let disposition =
            header_of(&headers, header::CONTENT_DISPOSITION).expect("content disposition");
        assert!(disposition.starts_with("inline; filename="));
        assert!(
            disposition.ends_with(".webp\""),
            "the filename is ours: {disposition}"
        );
    }

    #[test]
    fn last_modified_is_an_http_date() {
        let headers = image_headers("\"tag\"", 1, Some(SystemTime::UNIX_EPOCH));
        assert_eq!(
            header_of(&headers, header::LAST_MODIFIED),
            Some("Thu, 01 Jan 1970 00:00:00 GMT")
        );
    }

    /// The tag is derived from content and variant, so two variants of one
    /// photo never collide and one variant of two photos never does either.
    #[test]
    fn etags_are_content_derived() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);

        assert_eq!(etag(&a, VariantKind::Thumb), etag(&a, VariantKind::Thumb));
        assert_ne!(etag(&a, VariantKind::Thumb), etag(&a, VariantKind::Full));
        assert_ne!(etag(&a, VariantKind::Thumb), etag(&b, VariantKind::Thumb));
    }

    /// A variant that does not exist yet is an ordinary race — an upload
    /// returns before its variants do — and must never be a 500.
    #[test]
    fn a_pending_variant_is_not_a_server_error() {
        let status = availability(STATUS_PENDING)
            .expect_err("pending is not servable")
            .into_response()
            .status();
        assert_eq!(status, StatusCode::CONFLICT);

        for terminal in [STATUS_FAILED, "something-a-later-migration-adds"] {
            let status = availability(terminal)
                .expect_err("not servable")
                .into_response()
                .status();
            assert_eq!(status, StatusCode::NOT_FOUND, "{terminal}");
        }

        assert!(availability(STATUS_READY).is_ok());
    }

    /// The design's rejection branches, which `AppError` alone cannot express.
    #[test]
    fn domain_failures_keep_their_status() {
        let cases = [
            (PhotoError::TooLarge, StatusCode::PAYLOAD_TOO_LARGE),
            (PhotoError::NotAnImage, StatusCode::UNSUPPORTED_MEDIA_TYPE),
            (PhotoError::TooManyPixels, StatusCode::UNPROCESSABLE_ENTITY),
            (
                PhotoError::Undecodable("truncated".to_owned()),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                PhotoError::Unencodable("libwebp".to_owned()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(Rejection::from(err).into_response().status(), expected);
        }
    }

    /// 429 with a `Retry-After`, not a reset connection (T12).
    #[test]
    fn the_rate_limiter_refuses_with_a_retry_after() {
        let user = Uuid::now_v7();

        // Two per minute: two succeed, the third does not.
        assert!(take_upload_token(user, 2).is_ok());
        assert!(take_upload_token(user, 2).is_ok());

        let response = take_upload_token(user, 2)
            .expect_err("the third upload is over budget")
            .into_response();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .expect("retry-after is set");
        assert!(
            (1..=60).contains(&retry),
            "implausible retry-after: {retry}"
        );
    }

    /// Buckets are per user: one account exhausting its budget must not refuse
    /// anybody else's upload.
    #[test]
    fn the_rate_limiter_is_per_user() {
        let noisy = Uuid::now_v7();
        let quiet = Uuid::now_v7();

        assert!(take_upload_token(noisy, 1).is_ok());
        assert!(take_upload_token(noisy, 1).is_err());
        assert!(take_upload_token(quiet, 1).is_ok());
    }

    // -- the multipart reader ------------------------------------------------

    fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, content) in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"").as_bytes(),
            );
            if let Some(filename) = filename {
                body.extend_from_slice(format!("; filename=\"{filename}\"").as_bytes());
            }
            body.extend_from_slice(b"\r\nContent-Type: application/octet-stream\r\n\r\n");
            body.extend_from_slice(content);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        body
    }

    fn reader(boundary_value: &str, body: Vec<u8>, limit: u64) -> Multipart {
        Multipart::new(
            &format!("multipart/form-data; boundary={boundary_value}"),
            Body::from(body),
            limit,
        )
        .expect("a valid boundary")
    }

    /// The same body delivered `frame_size` bytes at a time, which is the shape
    /// a real network body has and an in-memory one never does.
    fn chunked_reader(boundary_value: &str, body: Vec<u8>, frame_size: usize) -> Multipart {
        let frames = body
            .chunks(frame_size)
            .map(Bytes::copy_from_slice)
            .collect();

        Multipart::from_source(
            &format!("multipart/form-data; boundary={boundary_value}"),
            Source::Frames(frames),
            u64::MAX,
        )
        .expect("a valid boundary")
    }

    async fn collect(multipart: &mut Multipart) -> Vec<(Option<String>, Vec<u8>)> {
        let mut parts = Vec::new();
        while let Some(part) = multipart.next_part().await.expect("a well-formed body") {
            let mut content = Vec::new();
            while let Some(chunk) = multipart.next_chunk().await.expect("a well-formed part") {
                content.extend_from_slice(&chunk);
            }
            parts.push((part.filename, content));
        }
        parts
    }

    #[tokio::test]
    async fn several_files_in_one_request_are_read_in_order() {
        let raw = multipart_body(
            "pv",
            &[
                ("caption", None, b"not a file"),
                ("files", Some("one.jpg"), b"\xff\xd8\xff\xe0one"),
                ("files", Some("two.png"), b"\x89PNGtwo"),
            ],
        );

        let parts = collect(&mut reader("pv", raw, 1 << 20)).await;

        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].0, None, "an ordinary field has no filename");
        assert_eq!(parts[1].0.as_deref(), Some("one.jpg"));
        assert_eq!(parts[1].1, b"\xff\xd8\xff\xe0one");
        assert_eq!(parts[2].1, b"\x89PNGtwo");
    }

    /// The case a naive split on the boundary gets wrong: image bytes that
    /// happen to contain the boundary text without the CRLF that precedes a
    /// real delimiter.
    #[tokio::test]
    async fn a_boundary_inside_the_data_is_not_a_delimiter() {
        let content = b"before--pv still the same file after".to_vec();
        let raw = multipart_body("pv", &[("files", Some("a.bin"), &content)]);

        let parts = collect(&mut reader("pv", raw, 1 << 20)).await;

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].1, content);
    }

    /// Every delimiter, header line and part boundary can land across a frame
    /// boundary. Sweeping the frame size exercises all of those splits, and
    /// one-byte frames exercise every one of them at once.
    #[tokio::test]
    async fn a_delimiter_split_across_frames_still_matches() {
        let content: Vec<u8> = (0u8..=255).cycle().take(3000).collect();
        let raw = multipart_body(
            "pv-boundary",
            &[
                ("files", Some("a.bin"), &content),
                ("files", Some("b.bin"), b"second"),
            ],
        );

        for frame_size in [1, 2, 3, 7, 13, 64, 1000] {
            let parts = collect(&mut chunked_reader("pv-boundary", raw.clone(), frame_size)).await;

            assert_eq!(parts.len(), 2, "frame size {frame_size}");
            assert_eq!(parts[0].0.as_deref(), Some("a.bin"), "{frame_size}");
            assert_eq!(parts[0].1, content, "frame size {frame_size}");
            assert_eq!(parts[1].1, b"second", "frame size {frame_size}");
        }
    }

    /// T12's mid-stream half: a body that exceeds the cap is refused with a 413
    /// even though nothing declared a `Content-Length`.
    #[tokio::test]
    async fn an_oversized_body_is_refused_with_413() {
        let raw = multipart_body("pv", &[("files", Some("a.bin"), &vec![0u8; 4096])]);
        let mut multipart = reader("pv", raw, 512);

        let status = loop {
            match multipart.next_part().await {
                Ok(Some(_)) => while let Ok(Some(_)) = multipart.next_chunk().await {},
                Ok(None) => panic!("the whole body was accepted"),
                Err(rejection) => break rejection.into_response().status(),
            }
        };

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// A truncated upload must not be stored as though it were whole.
    #[tokio::test]
    async fn a_body_with_no_closing_delimiter_is_refused() {
        let mut raw = multipart_body("pv", &[("files", Some("a.bin"), b"0123456789")]);
        raw.truncate(raw.len() - 12);

        let mut multipart = reader("pv", raw, 1 << 20);
        multipart.next_part().await.expect("headers are intact");

        let result = loop {
            match multipart.next_chunk().await {
                Ok(Some(_)) => continue,
                other => break other,
            }
        };

        assert!(result.is_err(), "a truncated body was accepted as complete");
    }

    #[test]
    fn only_a_multipart_content_type_is_accepted() {
        assert_eq!(
            boundary("multipart/form-data; boundary=abc").as_deref(),
            Some("abc")
        );
        assert_eq!(
            boundary("multipart/form-data; charset=utf-8; boundary=\"a b\"").as_deref(),
            Some("a b")
        );
        assert_eq!(boundary("application/json"), None);
        assert_eq!(boundary("multipart/form-data"), None);
        assert_eq!(boundary("multipart/form-data; boundary="), None);
        assert_eq!(
            boundary(&format!("multipart/form-data; boundary={}", "x".repeat(71))),
            None
        );
    }

    #[test]
    fn find_locates_a_needle_only_where_it_is() {
        assert_eq!(find(b"hello world", b"world"), Some(6));
        assert_eq!(find(b"aaab", b"aab"), Some(1));
        assert_eq!(find(b"abc", b"abcd"), None);
        assert_eq!(find(b"", b"a"), None);
        assert_eq!(find(b"abc", b""), None);
    }
}
