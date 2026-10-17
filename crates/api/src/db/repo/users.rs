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
         RETURNING id, display_name, avatar_url, created_at",
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
        "SELECT id, display_name, avatar_url, created_at FROM users WHERE id = $1",
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
        "SELECT u.id, u.display_name, u.avatar_url, u.created_at FROM users u
         JOIN profile_names n ON n.user_id = u.id WHERE n.name = $1",
        name,
    )
    .fetch_optional(executor)
    .await
}
