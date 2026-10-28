//! Administrator-only account summaries and confirmed UI actions. Metadata here
//! does not grant access to private photo content or exact coordinates.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    db::repo::users,
    error::{AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(list))
        .route("/api/admin/users/{id}", get(detail))
        .route("/api/admin/users/{id}/actions", post(act))
}

/// Read the role on every request; a cached browser role never authorizes access.
pub(super) async fn authorize(connection: &mut PgConnection, id: Uuid) -> AppResult<()> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_site_admin AND NOT suspended)",
    )
    .bind(id)
    .fetch_one(connection)
    .await?;
    if allowed {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[derive(Default, Deserialize)]
struct Page {
    before: Option<Uuid>,
    limit: Option<i64>,
}
impl Page {
    fn limit(&self) -> AppResult<i64> {
        let limit = self.limit.unwrap_or(25);
        if !(1..=100).contains(&limit) {
            return Err(AppError::BadRequest(
                "limit must be between 1 and 100".into(),
            ));
        }
        Ok(limit)
    }
}

#[derive(Serialize)]
struct AdminUser {
    id: Uuid,
    display_name: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    is_site_admin: bool,
    suspended: bool,
    voyage_count: i64,
    photo_count: i64,
    storage_bytes: i64,
}

impl From<users::AccountSummary> for AdminUser {
    fn from(account: users::AccountSummary) -> Self {
        Self {
            id: account.id,
            display_name: account.display_name,
            created_at: account.created_at,
            is_site_admin: account.is_site_admin,
            suspended: account.suspended,
            voyage_count: account.voyage_count,
            photo_count: account.photo_count,
            storage_bytes: 0,
        }
    }
}

/// Storage totals may be cached for 60 seconds; role and count queries remain current.
async fn measure_storage(state: &AppState, accounts: &mut [AdminUser]) -> AppResult<()> {
    let ids: Vec<_> = accounts.iter().map(|user| user.id).collect();
    let totals = state
        .account_storage
        .totals(&state.pool, &state.config.photos.storage_root, &ids)
        .await?;
    for account in accounts {
        account.storage_bytes = totals[&account.id];
    }
    Ok(())
}

#[derive(Serialize)]
struct UserPage {
    users: Vec<AdminUser>,
    next_before: Option<Uuid>,
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(page): Query<Page>,
) -> AppResult<Json<UserPage>> {
    authorize(&mut *state.pool.acquire().await?, user.user_id).await?;
    let limit = page.limit()?;
    let mut users: Vec<AdminUser> =
        users::account_summaries(&state.pool, None, page.before, limit + 1)
            .await?
            .into_iter()
            .map(AdminUser::from)
            .collect();
    let next_before = if users.len() > limit as usize {
        users.pop();
        users.last().map(|user| user.id)
    } else {
        None
    };
    measure_storage(&state, &mut users).await?;
    Ok(Json(UserPage { users, next_before }))
}

#[derive(Serialize, sqlx::FromRow)]
struct AdminVoyage {
    id: Uuid,
    title: String,
    description: Option<String>,
    published: bool,
}
#[derive(Serialize)]
struct UserDetail {
    user: AdminUser,
    voyages: Vec<AdminVoyage>,
    next_before: Option<Uuid>,
}

async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> AppResult<Json<UserDetail>> {
    authorize(&mut *state.pool.acquire().await?, user.user_id).await?;
    let limit = page.limit()?;
    let mut account: AdminUser = users::account_summaries(&state.pool, Some(id), None, 1)
        .await?
        .pop()
        .ok_or(AppError::NotFound)?
        .into();
    measure_storage(&state, std::slice::from_mut(&mut account)).await?;
    let mut voyages: Vec<AdminVoyage> = sqlx::query_as("SELECT id,title,description,published FROM voyages WHERE owner_id=$1 AND ($2::uuid IS NULL OR id<$2) ORDER BY id DESC LIMIT $3")
        .bind(id).bind(page.before).bind(limit+1).fetch_all(&state.pool).await?;
    let next_before = if voyages.len() > limit as usize {
        voyages.pop();
        voyages.last().map(|voyage| voyage.id)
    } else {
        None
    };
    Ok(Json(UserDetail {
        user: account,
        voyages,
        next_before,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Suspend,
    Enable,
    Promote,
    Delete,
    UnpublishAll,
    UnpublishVoyage { voyage_id: Option<Uuid> },
}

/// Serialize administrator mutations and recheck the actor inside the transaction.
/// This prevents two administrators concurrently removing each other's access.
/// Account deletion locks voyages before content, matching the upload/delete order.
async fn act(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(action): Json<Action>,
) -> AppResult<StatusCode> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(1919510126, 1)")
        .execute(&mut *tx)
        .await?;
    authorize(&mut tx, user.user_id).await?;
    let target: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    target.ok_or(AppError::NotFound)?;
    if id == user.user_id && matches!(action, Action::Suspend | Action::Delete) {
        return Err(AppError::Conflict(
            "You cannot suspend or delete your own administrator account.".into(),
        ));
    }
    match action {
        Action::Suspend | Action::Enable => {
            let suspended = matches!(action, Action::Suspend);
            sqlx::query("UPDATE users SET suspended=$2,updated_at=now() WHERE id=$1")
                .bind(id)
                .bind(suspended)
                .execute(&mut *tx)
                .await?;
            // Re-enabling also discards any sessions inserted outside normal sign-in.
            crate::auth::session::revoke_all_for_user(&mut *tx, id).await?;
            sqlx::query("DELETE FROM auth_attempts WHERE link_user_id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        Action::Promote => {
            sqlx::query("UPDATE users SET is_site_admin=true,updated_at=now() WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        Action::UnpublishAll => {
            sqlx::query("UPDATE voyages SET published=false,published_at=NULL,updated_at=now() WHERE owner_id=$1").bind(id).execute(&mut *tx).await?;
        }
        Action::UnpublishVoyage { voyage_id } => {
            let voyage_id =
                voyage_id.ok_or_else(|| AppError::BadRequest("voyage_id is required".into()))?;
            let result = sqlx::query("UPDATE voyages SET published=false,published_at=NULL,updated_at=now() WHERE owner_id=$1 AND id=$2")
                .bind(id).bind(voyage_id).execute(&mut *tx).await?;
            if result.rows_affected() == 0 {
                return Err(AppError::NotFound);
            }
        }
        Action::Delete => {
            crate::accounts::deletion::remove(&mut tx, id).await?;
        }
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
