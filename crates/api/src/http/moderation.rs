//! Private reports, moderator review, and account-scoped blocking.
use crate::{AppError, AppResult, AppState, auth::CurrentUser, db::repo::users};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgExecutor;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me/blocks", get(blocks))
        .route("/api/me/blocks/{id}", put(block).delete(unblock))
        .route("/api/reports", post(report))
        .route("/api/admin/reports", get(queue))
        .route("/api/admin/reports/{id}", get(detail))
        .route("/api/admin/reports/{id}/actions", post(resolve))
}
fn private(body: impl Serialize) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

pub(crate) async fn can_view(
    executor: impl PgExecutor<'_>,
    viewer: Option<Uuid>,
    voyage: Uuid,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM voyages v WHERE v.id=$1 AND (v.owner_id=$2 OR (v.published AND NOT v.moderation_held AND NOT EXISTS(SELECT 1 FROM account_blocks b WHERE b.account_id=$2 AND b.blocked_id=v.owner_id))))").bind(voyage).bind(viewer).fetch_one(executor).await?)
}
pub(crate) async fn blocked(
    executor: impl PgExecutor<'_>,
    viewer: Option<Uuid>,
    owner: Uuid,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM account_blocks WHERE account_id=$1 AND blocked_id=$2)",
    )
    .bind(viewer)
    .bind(owner)
    .fetch_one(executor)
    .await?)
}
async fn blocks(State(state): State<AppState>, user: CurrentUser) -> AppResult<Response> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT blocked_id FROM account_blocks WHERE account_id=$1 ORDER BY blocked_id",
    )
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(private(json!({"user_ids":ids})))
}
async fn block(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    change_block(&state, user.user_id, id, &headers, true).await
}
async fn unblock(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    change_block(&state, user.user_id, id, &headers, false).await
}
async fn change_block(
    state: &AppState,
    actor: Uuid,
    target: Uuid,
    headers: &HeaderMap,
    blocked: bool,
) -> AppResult<Response> {
    super::mutations::run(
        state,
        actor,
        headers,
        super::mutations::Request {
            method: if blocked { "PUT" } else { "DELETE" },
            path: format!("/api/me/blocks/{target}"),
            payload: json!(null),
            requires_revision: false,
        },
        async move |connection, _| change_block_in(connection, actor, target, blocked).await,
    )
    .await
}

async fn change_block_in(
    connection: &mut sqlx::PgConnection,
    actor: Uuid,
    target: Uuid,
    blocked: bool,
) -> AppResult<super::mutations::Outcome> {
    if actor == target {
        return Err(AppError::BadRequest(
            "You cannot block your own account.".into(),
        ));
    }
    if blocked {
        if users::find_by_id(&mut *connection, target).await?.is_none() {
            return Err(AppError::NotFound);
        }
        sqlx::query("INSERT INTO account_blocks(account_id,blocked_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(actor).bind(target).execute(&mut *connection).await?;
    } else {
        sqlx::query("DELETE FROM account_blocks WHERE account_id=$1 AND blocked_id=$2")
            .bind(actor)
            .bind(target)
            .execute(&mut *connection)
            .await?;
    }
    Ok(super::mutations::Outcome::empty(
        StatusCode::NO_CONTENT,
        target,
        target,
        "block",
        0,
    ))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    target: String,
    reason: String,
    details: String,
}
async fn report(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Json(input): Json<Submission>,
) -> AppResult<Response> {
    let payload = json!(input);
    super::mutations::run(
        &state,
        user.user_id,
        &headers,
        super::mutations::Request {
            method: "POST",
            path: "/api/reports".into(),
            payload,
            requires_revision: false,
        },
        async move |connection, _| create_report(connection, user.user_id, input).await,
    )
    .await
}

async fn create_report(
    connection: &mut sqlx::PgConnection,
    actor: Uuid,
    input: Submission,
) -> AppResult<super::mutations::Outcome> {
    let reason = input.reason.trim();
    let details = input.details.trim();
    if reason.is_empty() || reason.chars().count() > 100 || details.chars().count() > 2000 {
        return Err(AppError::BadRequest(
            "Choose a reason of at most 100 characters and details of at most 2,000 characters."
                .into(),
        ));
    }
    let (kind, raw) = input.target.split_once(':').ok_or_else(|| {
        AppError::BadRequest("Choose a voyage, photo or traveler to report.".into())
    })?;
    let target =
        Uuid::parse_str(raw).map_err(|_| AppError::BadRequest("Invalid report target.".into()))?;
    let (owner,voyage): (Uuid,Option<Uuid>)= match kind {
   "voyage"=>sqlx::query_as("SELECT owner_id,id FROM voyages WHERE id=$1 AND published AND NOT moderation_held").bind(target).fetch_optional(&mut *connection).await?.ok_or(AppError::NotFound)?,
   "photo"=>sqlx::query_as("SELECT v.owner_id,v.id FROM photos p JOIN voyages v ON v.id=p.voyage_id WHERE p.id=$1 AND v.published AND NOT v.moderation_held").bind(target).fetch_optional(&mut *connection).await?.ok_or(AppError::NotFound)?,
   "user"=>(users::find_by_id(&mut *connection,target).await?.ok_or(AppError::NotFound)?.id,None),
   _=>return Err(AppError::BadRequest("Unsupported report target.".into()))
  };
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 741))")
        .bind(actor.to_string())
        .execute(&mut *connection)
        .await?;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM content_reports WHERE reporter_id=$1 AND created_at>now()-interval '1 day'").bind(actor).fetch_one(&mut *connection).await?;
    if count >= 20 {
        return Err(AppError::RateLimited { retry_after: 86400 });
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO content_reports(id,reporter_id,target_owner_id,voyage_id,target,reason,details) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(actor).bind(owner).bind(voyage).bind(input.target).bind(reason).bind(details).execute(&mut *connection).await?;
    super::mutations::Outcome::json(
        StatusCode::CREATED,
        id,
        voyage.unwrap_or(owner),
        "report",
        0,
        json!({"id":id,"status":"open"}),
    )
}

#[derive(Default, Deserialize)]
struct Page {
    before: Option<Uuid>,
    limit: Option<i64>,
}
#[derive(Serialize, sqlx::FromRow)]
struct Report {
    id: Uuid,
    reporter_id: Option<Uuid>,
    target_owner_id: Option<Uuid>,
    voyage_id: Option<Uuid>,
    target: String,
    reason: String,
    details: String,
    created_at: DateTime<Utc>,
    status: String,
    moderator_id: Option<Uuid>,
    moderator_note: Option<String>,
    resolved_at: Option<DateTime<Utc>>,
}
async fn queue(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(page): Query<Page>,
) -> AppResult<Response> {
    super::admin::authorize(&mut *state.pool.acquire().await?, user.user_id).await?;
    let limit = page.limit.unwrap_or(25);
    if !(1..=100).contains(&limit) {
        return Err(AppError::BadRequest(
            "limit must be between 1 and 100".into(),
        ));
    }
    let mut reports: Vec<Report> = sqlx::query_as(
        "SELECT * FROM content_reports WHERE ($1::uuid IS NULL OR id<$1) ORDER BY id DESC LIMIT $2",
    )
    .bind(page.before)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let next = if reports.len() > limit as usize {
        reports.pop();
        reports.last().map(|report| report.id)
    } else {
        None
    };
    Ok(private(json!({"reports":reports,"next_before":next})))
}
async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    super::admin::authorize(&mut *state.pool.acquire().await?, user.user_id).await?;
    let report: Report = sqlx::query_as("SELECT * FROM content_reports WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(private(report))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolution {
    action: String,
    note: String,
}
async fn resolve(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(input): Json<Resolution>,
) -> AppResult<StatusCode> {
    if !matches!(input.action.as_str(), "hold" | "release" | "dismiss")
        || input.note.trim().is_empty()
        || input.note.chars().count() > 2000
    {
        return Err(AppError::BadRequest(
            "Choose a review action and include a note of at most 2,000 characters.".into(),
        ));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(1919510126, 1)")
        .execute(&mut *tx)
        .await?;
    super::admin::authorize(&mut tx, user.user_id).await?;
    let report: Report = sqlx::query_as("SELECT * FROM content_reports WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    if input.action != "dismiss" {
        let owner = report.target_owner_id.ok_or(AppError::NotFound)?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
        let held = input.action == "hold";
        // Holding also unpublishes and invalidates armed consent. Release never republishes.
        sqlx::query("UPDATE voyages SET moderation_held=$3,published=CASE WHEN $3 THEN false ELSE published END,updated_at=now() WHERE owner_id=$1 AND ($2::uuid IS NULL OR id=$2)").bind(owner).bind(report.voyage_id).bind(held).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE content_reports SET status=$2,moderator_id=$3,moderator_note=$4,resolved_at=now() WHERE id=$1").bind(id).bind(if input.action=="hold"{"held"}else{"resolved"}).bind(user.user_id).bind(input.note.trim()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
