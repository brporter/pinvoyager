//! Voyage uploads, owner metadata, photo deletion, and authorized WebP delivery.
//!
//! Originals are never served. Variant responses use a server-chosen MIME
//! type, nosniff, a controlled filename, and private revalidation so origin
//! authorization applies after publication changes.

use std::path::Path as FsPath;
use std::time::{Instant, SystemTime};

use axum::body::Body;
use axum::extract::multipart::{Field, MultipartError};
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, State};
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

/// Revalidate every use, including after publication or ownership changes.
/// Shared caches must never reuse a response authorized for one caller.
const IMAGE_CACHE_CONTROL: &str = "private, no-cache, must-revalidate";

/// Registers upload, owner metadata, deletion, and variant routes. Variant
/// names are parsed into the supported enum before constructing storage paths.
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

/// Returns 404 for a missing voyage or one not owned by the caller.
/// Database failures propagate.
async fn owned_voyage(state: &AppState, user: &CurrentUser, id: Uuid) -> Result<(), Rejection> {
    let voyage = voyages_repo::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if voyage.owner_id != user.user_id {
        return Err(AppError::NotFound.into());
    }
    Ok(())
}

#[tracing::instrument(name = "photo.upload", skip_all, fields(voyage.id = %voyage_id), err(Debug))]
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

/// Streams multipart files into temporary storage and accepts validated photos.
///
/// Enforces the request size limit, detects format from bytes, and checks image
/// dimensions before decoding. Client filenames and MIME types do not choose the
/// format. Each accepted file is saved and queued independently; a later failure
/// does not roll back files already accepted in the same request.
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
    state
        .upload_limiter
        .take_at(user.user_id, limits.upload_rate_per_minute, Instant::now())
        .map_err(Rejection::rate_limited)?;

    // The declared length, before a byte of the body is read: a truthful client
    // gets its 413 immediately. A lying one — or a chunked body, which declares
    // no length at all — hits the same limit mid-stream in the multipart extractor,
    // which is why both checks exist.
    if let Some(declared) = content_length(&headers)
        && declared > limits.max_upload_bytes
    {
        return Err(Rejection::too_large());
    }

    let upload_key = headers
        .get("idempotency-key")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| AppError::BadRequest("Idempotency-Key must be a UUID.".to_owned()))
        })
        .transpose()?;
    let mut multipart = read_multipart(headers, body, limits.max_upload_bytes).await?;

    // Temporaries live under the storage root rather than in the system temp
    // directory, so committing an upload is a rename within one filesystem. A
    // rename across filesystems is a copy, which would double the write cost of
    // every upload and stop being atomic.
    let temp_dir = limits.storage_root.join("tmp");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(PhotoError::Storage)?;

    let mut created = Vec::new();

    while let Some(mut part) = multipart.next_field().await.map_err(multipart_error)? {
        check_part_headers(&part)?;
        if part.file_name().is_none() {
            // Not a file part. Skipped rather than refused: the upload form may
            // grow ordinary fields, and this endpoint has no use for them yet.
            // `next_field` drains the body of a part the caller did not read.
            continue;
        }

        let key = upload_key.map(|key| format!("{key}:{}", created.len()));
        created.push(
            ingest(
                &state,
                user.user_id,
                voyage_id,
                &mut part,
                &temp_dir,
                key.as_deref(),
            )
            .await?,
        );
    }

    if created.is_empty() {
        return Err(AppError::BadRequest("The request contained no files.".to_owned()).into());
    }

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse { photos: created }),
    ))
}

/// Ingests one file and attempts to remove its temporary path after success
/// or failure. Cancellation drops a shared lease that schedules removal as well.
/// Cleanup failures are logged without replacing the ingest result.
async fn ingest(
    state: &AppState,
    owner_id: Uuid,
    voyage_id: Uuid,
    multipart: &mut Field<'_>,
    temp_dir: &FsPath,
    upload_key: Option<&str>,
) -> Result<PhotoResponse, Rejection> {
    // Content addressing means no caller-supplied string ever reaches the
    // filesystem — including here, where the name is a fresh uuid.
    let (temporary, file) = crate::photos::temporary::TemporaryUpload::create(temp_dir).await?;

    let result = ingest_part(
        state, owner_id, voyage_id, multipart, &temporary, file, upload_key,
    )
    .await;

    discard_failed_upload(temporary.path()).await;

    result
}

async fn discard_failed_upload(path: &FsPath) {
    // Attempt temporary-file cleanup without replacing the upload result.
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            let error_message = err.to_string();
            tracing::warn!(error = %error_message, "could not remove a rejected upload's temporary file");
        }
    }
}

#[tracing::instrument(name = "photo.ingest", skip_all, fields(voyage.id = %voyage_id, photo.id = tracing::field::Empty, photo.bytes = tracing::field::Empty), err(Debug))]
async fn ingest_part(
    state: &AppState,
    owner_id: Uuid,
    voyage_id: Uuid,
    multipart: &mut Field<'_>,
    temporary: &crate::photos::temporary::TemporaryUpload,
    mut file: tokio::fs::File,
    upload_key: Option<&str>,
) -> Result<PhotoResponse, Rejection> {
    let limits = &state.config.photos;

    let temp_path = temporary.path();

    let mut header = Vec::with_capacity(SNIFF_BYTES);
    let mut format: Option<ImageFormat> = None;
    let mut byte_size: u64 = 0;

    while let Some(chunk) = multipart.chunk().await.map_err(multipart_error)? {
        byte_size += chunk.len() as u64;
        if byte_size > limits.max_upload_bytes {
            return Err(Rejection::too_large());
        }

        if format.is_none() && header.len() < SNIFF_BYTES {
            let take = (SNIFF_BYTES - header.len()).min(chunk.len());
            header.extend_from_slice(&chunk[..take]);

            // Format check, and the stored-XSS defense. The moment enough bytes exist
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
    let probe_file = temporary.clone();

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
        // Keep the temporary file and its lock alive if the request is canceled.
        let probe_path = probe_file.path();
        // Dimension check, before any full decode: a 50,000 × 50,000 PNG is a few
        // hundred kilobytes on disk and about 10 GB decoded, so only a pixel
        // count catches it — a byte-size limit never will.
        let (width, height) = tracing::info_span!("photo.validate").in_scope(|| {
            let dimensions = validate::probe_dimensions(probe_path, format)?;
            validate::check_pixel_budget(dimensions.0, dimensions.1, max_megapixels)?;
            Ok::<_, PhotoError>(dimensions)
        })?;

        // Only now, on bytes already known to be an image of a sane size.
        let hash =
            tracing::info_span!("photo.hash").in_scope(|| ContentHash::of_file(probe_path))?;
        // Read from the original. Absent EXIF is the normal case, not an error.
        let exif = tracing::info_span!("photo.exif").in_scope(|| exif::extract(probe_path));

        Ok::<_, PhotoError>((width, height, hash, exif))
    })
    .await?;

    let (width, height, hash, exif) = probed?;

    // Reserve before opening a transaction: backpressure must not consume a
    // database connection. After commit, sending through this permit cannot
    // wait or report a closed queue after the photo has already been saved.
    let permit = state
        .variants
        .reserve()
        .await
        .map_err(|_| AppError::ServiceUnavailable)?;
    let mut tx = state.pool.begin().await?;
    // Lock the account before its voyage, matching administrator deletion, and
    // reject an upload whose account was suspended while its bytes arrived.
    crate::db::repo::users::require_active(&mut tx, owner_id).await?;
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM voyages WHERE id=$1 AND owner_id=$2 FOR UPDATE")
            .bind(voyage_id)
            .bind(owner_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound.into());
    }
    if let Some(key) = upload_key {
        let existing: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM photos WHERE voyage_id=$1 AND upload_key=$2")
                .bind(voyage_id)
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(id) = existing {
            let photo = photos::find_by_id(&mut *tx, id)
                .await?
                .ok_or(AppError::NotFound)?;
            if photo.content_hash != hash.as_str() {
                return Err(AppError::Conflict(
                    "This upload key was already used for different bytes.".to_owned(),
                )
                .into());
            }
            return Ok(photo.into());
        }
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
            // Preserve exact EXIF coordinates; public responses apply granularity later.
            latitude: exif.latitude,
            longitude: exif.longitude,
        },
    )
    .await?;

    if let Some(key) = upload_key {
        sqlx::query("UPDATE photos SET upload_key=$1 WHERE id=$2")
            .bind(key)
            .bind(photo.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    tracing::Span::current().record("photo.id", photo.id.to_string());
    tracing::Span::current().record("photo.bytes", byte_size);

    permit.send(VariantJob::new(photo.id).with_voyage(voyage_id));

    Ok(PhotoResponse::from(photo))
}

/// The magic-byte check. Never the extension, never the client's `Content-Type`.
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
/// Exact coordinates appear only in owner metadata. Public voyage and discovery
/// responses apply the voyage's granularity before serialization.
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

/// Loads exact owner metadata. Missing and foreign photos both return 404
/// so this endpoint does not confirm the existence of another owner's photo.
async fn owned(state: &AppState, user: &CurrentUser, id: Uuid) -> Result<Photo, Rejection> {
    let photo = photos::find_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;

    if photo.owner_id != user.user_id {
        return Err(AppError::NotFound.into());
    }

    Ok(photo)
}

/// Loads an image the caller owns or that belongs to a published voyage.
/// Returns 404 for missing or inaccessible photos. Used for variant bytes only;
/// exact metadata requires ownership. Database failures propagate.
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

/// Authorizes and serves a WebP variant with controlled response headers.
/// Checks access before conditional validators. Pending variants return 409;
/// missing or failed variants return 404. Successful bodies stream from disk.
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

    let hash = ContentHash::try_from(photo.content_hash)?;
    let path = storage::variant_path(&state.config.photos.storage_root, &hash, kind);

    let (file, size, modified) = match variant_body(&path).await {
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

    let image_headers = image_headers(&etag, size, modified);

    // A 304 carries the validators and no body — a body on a 304 is a protocol
    // error, and not re-sending the image is the entire point.
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response()
    };
    response.headers_mut().extend(image_headers);

    Ok(response)
}

/// Open without reading the body, so conditional requests avoid image I/O.
async fn variant_body(
    path: &FsPath,
) -> Result<(tokio::fs::File, u64, Option<SystemTime>), std::io::Error> {
    let file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "variant is not a regular file",
        ));
    }
    Ok((file, metadata.len(), metadata.modified().ok()))
}

/// Returns success for ready variants, a retryable 409 for pending variants,
/// and 404 for failed or unrecognized processing states.
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

/// Builds an ETag from the original content hash and variant name.
/// It does not include encoder settings or hash the encoded output.
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

/// Deletes an owned photo and schedules content cleanup in the same transaction.
/// Background maintenance locks the content and removes files only if no photo
/// still references them. The request returns after the database commit.
/// Missing or foreign photos return 404.
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
        crate::photos::cleanup::schedule(&mut tx, &hash).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Axum streams fields while its body limit counts all multipart framing.
async fn read_multipart(
    headers: HeaderMap,
    body: Body,
    limit: u64,
) -> Result<Multipart, Rejection> {
    let mut request = axum::http::Request::new(body);
    *request.headers_mut() = headers;
    DefaultBodyLimit::max(usize::try_from(limit).unwrap_or(usize::MAX)).apply(&mut request);
    Multipart::from_request(request, &()).await.map_err(|_| {
        AppError::BadRequest("Expected a multipart/form-data request body.".into()).into()
    })
}

fn check_part_headers(field: &Field<'_>) -> Result<(), Rejection> {
    // Keep part headers small even though the entire request also has a cap.
    let size: usize = field
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
        .sum();
    if size > 4096 {
        return Err(AppError::BadRequest("Multipart part headers exceed 4 KB.".into()).into());
    }
    Ok(())
}

fn multipart_error(error: MultipartError) -> Rejection {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        Rejection::too_large()
    } else {
        AppError::BadRequest("The multipart request body is malformed.".into()).into()
    }
}

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

    /// The magic-byte check. A `.jpg` whose bytes are HTML lands here, and this
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
