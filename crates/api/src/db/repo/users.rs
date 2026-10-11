//! Users — the account. Owns voyages. Holds no credential of any kind (D1).

use sqlx::PgExecutor;
use uuid::Uuid;

/// Column set is deliberately narrower than the table: `updated_at` exists in
/// the schema but nothing reads it yet, and a field no caller uses is a field
/// every query has to keep selecting.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Creates an account.
///
/// There is no separate registration path — D2 makes a first successful
/// sign-in the signup — so the callback handler calls this,
/// in the same transaction that inserts the identity row. That is why this
/// takes an executor rather than a pool.
pub async fn create(
    executor: impl PgExecutor<'_>,
    display_name: Option<&str>,
    avatar_url: Option<&str>,
) -> Result<User, sqlx::Error> {
    // v7 rather than v4: the ids are time-ordered, so inserts land at the right
    // edge of the primary key index instead of scattering across it. Generated
    // here rather than by the database so the caller holds the id before the
    // round trip completes and can build dependent rows without a RETURNING
    // read-back on every one.
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
