//! Identities — one row per linked provider, mapping (provider, subject) to a
//! user. Unique on (provider, subject).
//!
//! D3: matching is on `subject`, never on email. Two providers can return the
//! same address for the same human and not every provider verifies ownership,
//! so email-based matching is an account-takeover vector.

use sqlx::PgExecutor;
use uuid::Uuid;

/// Provider names encoded as lowercase text: `google`, `microsoft`, or `apple`.
/// The sqlx text type annotation lets queries decode the column into this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum Provider {
    Google,
    Microsoft,
    Apple,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Identity {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: Provider,
    pub subject: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Links a provider and subject to an account. Returns database errors,
/// including a unique violation when that identity is already linked.
pub async fn create(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    provider: Provider,
    subject: &str,
) -> Result<Identity, sqlx::Error> {
    let id = Uuid::now_v7();

    // `provider as "provider: Provider"` is what tells the macro to decode a
    // `text` column into the enum. Without the annotation it would insist on
    // `String`, since the column really is text (see the type docs above).
    sqlx::query_as!(
        Identity,
        r#"INSERT INTO identities (id, user_id, provider, subject)
           VALUES ($1, $2, $3, $4)
           RETURNING id, user_id, provider as "provider: Provider", subject, created_at"#,
        id,
        user_id,
        provider as Provider,
        subject,
    )
    .fetch_one(executor)
    .await
}

/// The sign-in lookup: which account, if any, does this provider subject belong
/// to? Hits `identities_provider_subject_key` directly.
///
/// `Ok(None)` means "first sign-in" (D2 makes that a signup), not an error.
pub async fn find_by_provider_subject(
    executor: impl PgExecutor<'_>,
    provider: Provider,
    subject: &str,
) -> Result<Option<Identity>, sqlx::Error> {
    sqlx::query_as!(
        Identity,
        r#"SELECT id, user_id, provider as "provider: Provider", subject, created_at
           FROM identities
           WHERE provider = $1 AND subject = $2"#,
        provider as Provider,
        subject,
    )
    .fetch_optional(executor)
    .await
}
