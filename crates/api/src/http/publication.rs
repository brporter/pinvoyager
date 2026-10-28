//! Owner publication review and cancellation; conversion workers finalize independently.
use crate::{
    AppError, AppResult, AppState,
    auth::CurrentUser,
    voyages::publication::{self, Review},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/voyages/{id}/publication-intents",
            post(register).get(list),
        )
        .route(
            "/api/voyages/{id}/publication-intents/{intent}",
            get(status).delete(cancel),
        )
}

async fn register(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(voyage): Path<Uuid>,
    headers: HeaderMap,
    Json(review): Json<Review>,
) -> AppResult<Response> {
    let payload = json!(review);
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "POST",
            path: format!("/api/voyages/{voyage}/publication-intents"),
            payload,
            requires_revision: true,
        },
        async move |connection, expected| {
            let status =
                publication::register(connection, user.user_id, voyage, expected, review).await?;
            let revision = super::mutations::revision(connection, voyage, None).await?;
            super::mutations::Outcome::json(
                StatusCode::CREATED,
                status.id,
                voyage,
                "publication",
                revision,
                status,
            )
        },
    )
    .await
}

async fn status(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((voyage, intent)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    let owned: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM voyages WHERE id=$1 AND owner_id=$2)")
            .bind(voyage)
            .bind(user.user_id)
            .fetch_one(&state.pool)
            .await?;
    if !owned {
        return Err(AppError::NotFound);
    }
    publication::finalize(&state.pool, voyage).await?;
    let mut connection = state.pool.acquire().await?;
    let status = publication::status(&mut connection, user.user_id, voyage, intent).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(status)).into_response())
}

async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((voyage, intent)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "DELETE",
            path: format!("/api/voyages/{voyage}/publication-intents/{intent}"),
            payload: json!(null),
            requires_revision: false,
        },
        async |connection, _| {
            let status = publication::cancel(connection, user.user_id, voyage, intent).await?;
            let revision = super::mutations::revision(connection, voyage, None).await?;
            super::mutations::Outcome::json(
                StatusCode::OK,
                intent,
                voyage,
                "publication",
                revision,
                status,
            )
        },
    )
    .await
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(voyage): Path<Uuid>,
) -> AppResult<Response> {
    let owned: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM voyages WHERE id=$1 AND owner_id=$2)")
            .bind(voyage)
            .bind(user.user_id)
            .fetch_one(&state.pool)
            .await?;
    if !owned {
        return Err(AppError::NotFound);
    }
    publication::finalize(&state.pool, voyage).await?;
    let mut connection = state.pool.acquire().await?;
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM publication_intents WHERE voyage_id=$1 AND owner_id=$2 AND status IN ('waiting_for_uploads','processing') ORDER BY id").bind(voyage).bind(user.user_id).fetch_all(&mut *connection).await?;
    let mut intents = Vec::new();
    for id in ids {
        intents.push(publication::status(&mut connection, user.user_id, voyage, id).await?);
    }
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"intents":intents})),
    )
        .into_response())
}
