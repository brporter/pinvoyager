//! D14: unique public handles and anonymous, owner-filtered voyage pages.
use crate::{
    AppState,
    auth::CurrentUser,
    db::repo::{users, voyages},
    error::{AppError, AppResult},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/profiles/{identifier}", get(show))
}

#[derive(Deserialize)]
pub(crate) struct UpdateName {
    display_name: String,
}

fn normalized_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_ascii_lowercase();
    let bytes = name.as_bytes();
    if !(3..=30).contains(&bytes.len())
        || !bytes[0].is_ascii_lowercase()
        || !bytes[bytes.len() - 1].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        return Err(AppError::BadRequest("Use 3–30 letters, digits, or hyphens; start with a letter and end with a letter or digit.".into()));
    }
    Ok(name)
}

pub(crate) async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(input): Json<UpdateName>,
) -> AppResult<StatusCode> {
    let name = normalized_name(&input.display_name)?;
    let mut tx = state.pool.begin().await?;
    // Serialize renames by the same account; the registry's unique key arbitrates
    // claims by different accounts. Keep every old name as an owned alias.
    sqlx::query_scalar!(
        "SELECT id FROM users WHERE id = $1 FOR UPDATE",
        user.user_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::Unauthorized)?;
    users::require_active(&mut tx, user.user_id).await?;
    // New claims insert; a same-owner alias performs a no-op update and returns
    // success. Foreign and permanently reserved names return no row.
    let claimed = sqlx::query_scalar!(
        "INSERT INTO profile_names (name, user_id) VALUES ($1, $2)
         ON CONFLICT (name) DO UPDATE SET user_id = profile_names.user_id
         WHERE profile_names.user_id = EXCLUDED.user_id RETURNING name",
        name,
        user.user_id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if claimed.is_none() {
        return Err(AppError::Conflict(
            "That display name is unavailable or reserved. Choose another.".into(),
        ));
    }
    sqlx::query!(
        "UPDATE users SET display_name = $2, updated_at = now() WHERE id = $1",
        user.user_id,
        name
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .is_some_and(|db| db.is_unique_violation())
        {
            AppError::Conflict("That display name is already in use. Choose another.".into())
        } else {
            AppError::Database(error)
        }
    })?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PageQuery {
    before: Option<String>,
    limit: Option<i64>,
}
#[derive(Serialize)]
struct PublicUser {
    id: uuid::Uuid,
    display_name: Option<String>,
    avatar_url: Option<String>,
    created_at: DateTime<Utc>,
}
#[derive(Serialize)]
struct PublicProfile {
    user: PublicUser,
    voyages: Vec<super::voyages::VoyageSummary>,
    next_before: Option<String>,
}

async fn show(
    State(state): State<AppState>,
    crate::auth::MaybeUser(viewer): crate::auth::MaybeUser,
    Path(identifier): Path<String>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<PublicProfile>> {
    let account = match uuid::Uuid::parse_str(&identifier) {
        Ok(id) => users::find_by_id(&state.pool, id).await?,
        Err(_) => {
            users::find_by_profile_name(&state.pool, &identifier.to_ascii_lowercase()).await?
        }
    }
    .ok_or(AppError::NotFound)?;
    if super::moderation::blocked(
        &state.pool,
        viewer.as_ref().map(|user| user.user_id),
        account.id,
    )
    .await?
    {
        return Err(AppError::NotFound);
    }
    let before = query
        .before
        .as_deref()
        .map(|value| {
            voyages::PublishedCursor::parse(value)
                .ok_or_else(|| AppError::BadRequest("Invalid pagination cursor.".into()))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(24).clamp(1, 100);
    let voyages = voyages::list_published_for_owner(&state.pool, account.id, before, limit).await?;
    let next_before = if voyages.len() as i64 == limit {
        voyages
            .last()
            .and_then(voyages::PublishedCursor::of)
            .map(|cursor| cursor.encode())
    } else {
        None
    };
    let voyages = super::voyages::summaries(&state.pool, voyages).await?;
    Ok(Json(PublicProfile {
        user: PublicUser {
            id: account.id,
            display_name: account.display_name,
            avatar_url: account.avatar_url,
            created_at: account.created_at,
        },
        voyages,
        next_before,
    }))
}
