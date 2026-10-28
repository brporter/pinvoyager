//! Explicit owner-only retention of stripped image variants; originals have no route.
use crate::{
    AppError, AppResult, AppState,
    auth::CurrentUser,
    db::repo::voyages,
    photos::{
        VariantKind,
        storage::{self, ContentHash},
    },
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;
const PERMISSION: &str = "owner_offline_copy_v1";
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/voyages/{id}/offline-manifest", get(manifest))
        .route("/api/photos/{id}/offline/{variant}", get(file))
}
#[derive(sqlx::FromRow, Serialize)]
struct Photo {
    id: Uuid,
    position: i32,
    width: i32,
    height: i32,
    captured_at: Option<DateTime<Utc>>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    #[serde(skip)]
    content_hash: String,
    #[serde(skip)]
    variant_status: String,
    #[sqlx(skip)]
    variants: Vec<Variant>,
}
#[derive(Default, Serialize)]
struct Variant {
    kind: &'static str,
    url: String,
    byte_count: u64,
    sha256: String,
}
#[derive(Serialize)]
struct Manifest {
    voyage_id: Uuid,
    owner_id: Uuid,
    revision: String,
    title: String,
    description: Option<String>,
    granularity: String,
    published: bool,
    retention_permission: &'static str,
    authorized_at: DateTime<Utc>,
    photos: Vec<Photo>,
}
async fn owned(pool: &PgPool, owner: Uuid, voyage: Uuid) -> AppResult<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM voyages WHERE id=$1 AND owner_id=$2)")
            .bind(voyage)
            .bind(owner)
            .fetch_one(pool)
            .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}
async fn manifest(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    owned(&state.pool, user.user_id, id).await?;
    let mut tx = state.pool.begin().await?;
    let revision: i64 =
        sqlx::query_scalar("SELECT revision FROM voyages WHERE id=$1 AND owner_id=$2 FOR SHARE")
            .bind(id)
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    let voyage = voyages::find_by_id(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut photos:Vec<Photo>=sqlx::query_as("SELECT id,position,width,height,captured_at,latitude::float8 AS latitude,longitude::float8 AS longitude,content_hash,variant_status FROM photos WHERE voyage_id=$1 ORDER BY position,id").bind(id).fetch_all(&mut *tx).await?;
    for photo in &mut photos {
        if photo.variant_status != "ready" {
            return Err(AppError::Conflict(
                "Wait for every photograph to finish processing before downloading.".into(),
            ));
        }
        let hash = ContentHash::try_from(photo.content_hash.clone())?;
        for kind in VariantKind::ALL {
            let path = storage::variant_path(&state.config.photos.storage_root, &hash, kind);
            let (byte_count, sha256) = crate::blocking::run_blocking(move || {
                let length = std::fs::metadata(&path).map_err(AppError::from_file)?.len();
                let hash = ContentHash::of_file(&path).map_err(|error| match error {
                    crate::photos::PhotoError::Storage(error) => AppError::from_file(error),
                    error => AppError::internal(error),
                })?;
                Ok((length, hash.as_str().to_owned()))
            })
            .await?;
            photo.variants.push(Variant {
                kind: kind.as_str(),
                url: format!("/api/photos/{}/offline/{}", photo.id, kind.as_str()),
                byte_count,
                sha256,
            });
        }
    }
    tx.commit().await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(Manifest {
            voyage_id: id,
            owner_id: user.user_id,
            revision: revision.to_string(),
            title: voyage.title,
            description: voyage.description,
            granularity: voyage.granularity,
            published: voyage.published,
            retention_permission: PERMISSION,
            authorized_at: Utc::now(),
            photos,
        }),
    )
        .into_response())
}
async fn file(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, variant)): Path<(Uuid, String)>,
) -> AppResult<Response> {
    let kind = VariantKind::parse(&variant).ok_or(AppError::NotFound)?;
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT content_hash FROM photos WHERE id=$1 AND owner_id=$2 AND variant_status='ready'",
    )
    .bind(id)
    .bind(user.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let hash = ContentHash::try_from(hash.ok_or(AppError::NotFound)?)?;
    let file = tokio::fs::File::open(storage::variant_path(
        &state.config.photos.storage_root,
        &hash,
        kind,
    ))
    .await
    .map_err(AppError::from_file)?;
    let metadata = file.metadata().await.map_err(AppError::internal)?;
    if !metadata.is_file() {
        return Err(AppError::NotFound);
    }
    let size = metadata.len();
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (
                header::HeaderName::from_static("x-pinvoyager-retention"),
                PERMISSION,
            ),
            (header::CONTENT_LENGTH, &size.to_string()),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}
