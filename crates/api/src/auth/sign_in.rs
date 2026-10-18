//! Shared account policy and session creation after identity verification or
//! the explicitly enabled development bypass. Accounts match provider/subject;
//! development requests may explicitly grant administration.

use uuid::Uuid;

#[cfg(debug_assertions)]
use crate::auth::ProviderKind;
use crate::auth::{
    VerifiedIdentity,
    session::{self, SessionToken},
};
use crate::db::repo::{
    identities::{self, Provider as DbProvider},
    users,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Client-visible message for rejected authentication and first-sign-in collisions.
pub(crate) const GENERIC_FAILURE: &str =
    "We could not complete sign-in. Starting again normally works.";

/// Identity accepted by a sign-in entry point. Production callers supply a verified
/// provider identity. The development variant cannot be constructed in release builds.
pub(crate) enum SignInIdentity<'a> {
    Verified(&'a VerifiedIdentity),
    #[cfg(debug_assertions)]
    Development {
        provider: ProviderKind,
        subject: &'a str,
        grant_admin: bool,
    },
}

/// Resolves or creates an account, rejects suspended accounts, records identity
/// activity, and creates an opaque session atomically. Debug-only development
/// requests may grant administration in the same transaction.
/// Returns the account ID and raw session token after commit. Concurrent first
/// sign-ins that collide return a generic retryable 400; other database failures
/// propagate. Development callers must first enforce `auth.dev_login`.
pub(crate) async fn sign_in(
    state: &AppState,
    identity: SignInIdentity<'_>,
) -> AppResult<(Uuid, SessionToken)> {
    #[cfg(debug_assertions)]
    let grant_admin = matches!(
        &identity,
        SignInIdentity::Development {
            grant_admin: true,
            ..
        }
    );
    let (kind, subject, avatar_url) = match identity {
        SignInIdentity::Verified(identity) => (
            identity.provider,
            identity.subject.as_str(),
            identity.avatar_url.as_deref(),
        ),
        #[cfg(debug_assertions)]
        SignInIdentity::Development {
            provider, subject, ..
        } => (provider, subject, None),
    };
    let provider = DbProvider::from(kind);

    // Account creation, identity activity, promotion, and session insertion
    // commit or roll back together. No provider calls occur in this transaction.
    let mut tx = state.pool.begin().await?;

    let user_id = match identities::find_by_provider_subject(&mut *tx, provider, subject).await? {
        Some(existing) => {
            // Acquire the write lock before checking status: concurrent
            // sign-ins must not both upgrade a shared lock to promote.
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
                .bind(existing.user_id)
                .execute(&mut *tx)
                .await?;
            users::require_active(&mut tx, existing.user_id).await?;
            // Both sign-in paths update identity activity without overwriting
            // the account's chosen display name or avatar.
            sqlx::query!(
                "UPDATE identities SET last_seen_at = now() WHERE id = $1",
                existing.id,
            )
            .execute(&mut *tx)
            .await?;

            existing.user_id
        }
        None => {
            // D2: the first sign-in *is* the signup. There is no separate
            // registration step to reach, which is why this is an insert rather
            // than a 404 pointing at one.
            let user = users::create(&mut *tx, None, avatar_url).await?;

            identities::create(&mut *tx, user.id, provider, subject)
                .await
                .map_err(first_sign_in_identity_error)?;

            user.id
        }
    };

    #[cfg(debug_assertions)]
    if grant_admin {
        sqlx::query("UPDATE users SET is_site_admin=true,updated_at=now() WHERE id=$1 AND NOT is_site_admin")
            .bind(user_id).execute(&mut *tx).await?;
    }
    let token = session::create(&mut *tx, user_id, &state.config.auth.session).await?;

    tx.commit().await?;
    Ok((user_id, token))
}

/// Roll back a colliding first sign-in and ask the caller to start again, allowing
/// the next attempt to find the winning identity. Other database errors propagate.
fn first_sign_in_identity_error(err: sqlx::Error) -> AppError {
    if matches!(&err, sqlx::Error::Database(db) if db.is_unique_violation()) {
        tracing::warn!(
            reason = "concurrent first sign-in for the same identity",
            "sign-in rejected"
        );
        AppError::BadRequest(GENERIC_FAILURE.to_owned())
    } else {
        err.into()
    }
}

#[cfg(test)]
#[path = "sign_in_tests.rs"]
mod tests;
