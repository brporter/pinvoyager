//! User accounts and profile names. Provider identities and session hashes
//! are stored separately.

use sqlx::PgExecutor;
use uuid::Uuid;

/// User fields read by account handlers; excludes the unused `updated_at` column.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_site_admin: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Creates an account with a UUIDv7 ID. Callers can include identity creation
/// in the same transaction. Database failures return an error.
pub async fn create(
    executor: impl PgExecutor<'_>,
    display_name: Option<&str>,
    avatar_url: Option<&str>,
) -> Result<User, sqlx::Error> {
    // Generate a time-ordered UUIDv7 for the primary key.
    let id = Uuid::now_v7();

    sqlx::query_as!(
        User,
        "INSERT INTO users (id, display_name, avatar_url)
         VALUES ($1, $2, $3)
         RETURNING id, display_name, avatar_url, created_at, is_site_admin",
        id,
        display_name,
        avatar_url,
    )
    .fetch_one(executor)
    .await
}

/// Looks up an account by id.
///
/// `Ok(None)` for "no such user" rather than an error: a missing row is an
/// ordinary outcome here (a stale session, a deleted account), and folding it
/// into `sqlx::Error` would push callers into matching on `RowNotFound`.
pub async fn find_by_id(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        "SELECT id, display_name, avatar_url, created_at, is_site_admin FROM users WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await
}

/// Resolve either a current handle or an alias; reserved/deleted names return no user.
pub async fn find_by_profile_name(
    executor: impl PgExecutor<'_>,
    name: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        "SELECT u.id, u.display_name, u.avatar_url, u.created_at, u.is_site_admin FROM users u
         JOIN profile_names n ON n.user_id = u.id WHERE n.name = $1",
        name,
    )
    .fetch_optional(executor)
    .await
}

/// Reject suspended or deleted accounts and hold a shared account lock until
/// caller's transaction commits, serializing account mutations with suspension/deletion.
pub async fn require_active(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
) -> crate::error::AppResult<()> {
    let suspended: Option<bool> =
        sqlx::query_scalar("SELECT suspended FROM users WHERE id=$1 FOR SHARE")
            .bind(id)
            .fetch_optional(connection)
            .await?;
    match suspended {
        Some(false) => Ok(()),
        _ => Err(crate::error::AppError::Forbidden),
    }
}

/// Account metadata for administrator views. Callers must authorize the actor.
#[derive(sqlx::FromRow)]
pub struct AccountSummary {
    pub id: Uuid,
    pub display_name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_site_admin: bool,
    pub suspended: bool,
    pub voyage_count: i64,
    pub photo_count: i64,
}

/// Loads a descending account page or a single account using the same projection.
/// Missing accounts return an empty vector; database failures return an error.
pub async fn account_summaries(
    executor: impl PgExecutor<'_>,
    id: Option<Uuid>,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AccountSummary>, sqlx::Error> {
    sqlx::query_as(
        "SELECT u.id, u.display_name, u.created_at, u.is_site_admin, u.suspended,
        (SELECT count(*) FROM voyages v WHERE v.owner_id=u.id) AS voyage_count,
        (SELECT count(*) FROM photos p WHERE p.owner_id=u.id) AS photo_count
        FROM users u WHERE ($1::uuid IS NULL OR u.id=$1) AND ($2::uuid IS NULL OR u.id<$2)
        ORDER BY u.id DESC LIMIT $3",
    )
    .bind(id)
    .bind(before)
    .bind(limit)
    .fetch_all(executor)
    .await
}
