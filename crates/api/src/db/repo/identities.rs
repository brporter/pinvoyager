//! Identities — one row per linked provider, mapping (provider, subject) to a
//! user. Unique on (provider, subject).
//!
//! D3: matching is on `subject`, never on email. Two providers can return the
//! same address for the same human and not every provider verifies ownership,
//! so email-based matching is an account-takeover vector.

use sqlx::PgExecutor;
use uuid::Uuid;

/// Maps to a plain `text` column, not a Postgres enum type.
///
/// `type_name = "text"` is what makes that work: the derive turns it into
/// `PgTypeInfo::with_name("text")`, which sqlx compares against the column's
/// type by name (case-insensitively, so it matches the built-in `TEXT`) and
/// resolves to an OID once per connection when binding. Reading it as an enum
/// on the Rust side while storing text keeps the provider list a code-level
/// decision — adding a fifth provider needs no `ALTER TYPE`, only the CHECK
/// constraint in the migration.
///
/// `rename_all` gives the lowercase spellings the migration's CHECK constraint
/// expects; the two must be changed together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum Provider {
    Google,
    Microsoft,
    Apple,
    Facebook,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Identity {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: Provider,
    pub subject: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Links a provider identity to an existing account.
///
/// A unique violation on `(provider, subject)` is the expected failure and is
/// returned as-is: it means the identity is already attached to some account,
/// and D3 requires that to be refused rather than merged. The callback decides what
/// the user sees; the repository's job is to not paper over it.
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
