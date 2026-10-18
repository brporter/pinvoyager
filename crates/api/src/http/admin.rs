//! Administrator-only account summaries and confirmed UI actions. Metadata here
//! does not grant access to private photo content or exact coordinates.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    db::repo::photos,
    error::{AppError, AppResult},
    photos::{VariantKind, cleanup, storage},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(list))
        .route("/api/admin/users/{id}", get(detail))
        .route("/api/admin/users/{id}/actions", post(act))
}

/// Read the role on every request; a cached browser role never authorizes access.
async fn authorize(connection: &mut PgConnection, id: Uuid) -> AppResult<()> {
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

#[derive(Serialize, sqlx::FromRow)]
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

/// Sum files currently present, once per content hash per account. Shared bytes
/// count for each owning account, so these totals are not site-wide disk usage.
/// Missing files contribute zero; other filesystem failures return an error.
async fn measure_storage(
    pool: &PgPool,
    root: &std::path::Path,
    accounts: &mut [AdminUser],
) -> AppResult<()> {
    let ids: Vec<Uuid> = accounts.iter().map(|user| user.id).collect();
    let hashes: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT DISTINCT owner_id, content_hash FROM photos WHERE owner_id=ANY($1)")
            .bind(&ids)
            .fetch_all(pool)
            .await?;
    let root = root.to_path_buf();
    let totals = crate::blocking::run_blocking(move || {
        let mut totals = std::collections::HashMap::<Uuid, i64>::new();
        for (owner, hash) in hashes {
            let hash = storage::ContentHash::try_from(hash)?;
            let paths = std::iter::once(storage::original_path(&root, &hash)).chain(
                VariantKind::ALL
                    .into_iter()
                    .map(|kind| storage::variant_path(&root, &hash, kind)),
            );
            for path in paths {
                match std::fs::metadata(path) {
                    Ok(metadata) => *totals.entry(owner).or_default() += metadata.len() as i64,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(AppError::Internal(Box::new(error))),
                }
            }
        }
        Ok(totals)
    })
    .await?;
    for account in accounts {
        account.storage_bytes = totals.get(&account.id).copied().unwrap_or_default();
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
    let mut users: Vec<AdminUser> = sqlx::query_as("SELECT u.id, u.display_name, u.created_at, u.is_site_admin, u.suspended,
    (SELECT count(*) FROM voyages v WHERE v.owner_id=u.id) AS voyage_count,
    (SELECT count(*) FROM photos p WHERE p.owner_id=u.id) AS photo_count,
    0::bigint AS storage_bytes FROM users u WHERE ($1::uuid IS NULL OR u.id<$1) ORDER BY u.id DESC LIMIT $2")
    .bind(page.before)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let next_before = if users.len() > limit as usize {
        users.pop();
        users.last().map(|user| user.id)
    } else {
        None
    };
    measure_storage(&state.pool, &state.config.photos.storage_root, &mut users).await?;
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
    let mut account: AdminUser = sqlx::query_as(
        "SELECT u.id, u.display_name, u.created_at, u.is_site_admin, u.suspended,
    (SELECT count(*) FROM voyages v WHERE v.owner_id=u.id) AS voyage_count,
    (SELECT count(*) FROM photos p WHERE p.owner_id=u.id) AS photo_count,
    0::bigint AS storage_bytes FROM users u WHERE u.id=$1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    measure_storage(
        &state.pool,
        &state.config.photos.storage_root,
        std::slice::from_mut(&mut account),
    )
    .await?;
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
    let mut hashes = Vec::<String>::new();
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
            sqlx::query("SELECT id FROM voyages WHERE owner_id=$1 ORDER BY id FOR UPDATE")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
            hashes = sqlx::query_scalar(
                "SELECT DISTINCT content_hash FROM photos WHERE owner_id=$1 ORDER BY content_hash",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            for hash in &hashes {
                photos::lock_content(&mut tx, hash).await?;
            }
            // Names remain reserved by the profile_names ON DELETE SET NULL rule.
            sqlx::query("DELETE FROM users WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            for hash in &hashes {
                cleanup::schedule(&mut tx, hash).await?;
            }
        }
    }
    tx.commit().await?;
    for hash in hashes {
        if let Err(error) =
            cleanup::clean(&state.pool, &state.config.photos.storage_root, &hash).await
        {
            tracing::warn!(%error,"account deletion committed; file cleanup will be retried");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
