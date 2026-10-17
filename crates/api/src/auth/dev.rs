//! Development sign-in without provider verification.
//!
//! A caller supplies a provider and subject and receives a session for that
//! identity, creating an account if needed. This bypass can impersonate existing
//! accounts. It is compiled only with debug assertions and additionally requires
//! `auth.dev_login` to be enabled.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse as _, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::auth::ProviderKind;
use crate::auth::session;
use crate::db::repo::identities::{self, Provider as DbProvider};
use crate::db::repo::users;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct DevLoginRequest {
    /// The provider subject to sign in as. An email-shaped string is the
    /// convenient thing to type; it is treated as an opaque identifier.
    pub subject: String,
    /// Which provider this forged identity claims to come from. Defaults to
    /// Google, which is only a convenience — nothing contacts it.
    #[serde(default)]
    pub provider: Option<String>,
    /// Legacy request field, accepted but ignored. Like real sign-in, a new
    /// account chooses its unique public name through PATCH /api/me (D14).
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Finds or creates the supplied provider identity and issues a session in one
/// transaction. Returns 404 when development login is disabled and propagates
/// database failures.
pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(request): Json<DevLoginRequest>,
) -> AppResult<(CookieJar, Response)> {
    if !state.config.auth.dev_login {
        // 404 rather than 403: a disabled bypass should be indistinguishable
        // from one that was never built.
        return Err(AppError::NotFound);
    }

    if request.subject.trim().is_empty() {
        return Err(AppError::BadRequest("subject must not be empty".to_owned()));
    }

    let kind = match request.provider.as_deref() {
        Some(name) => ProviderKind::parse(name)
            .ok_or_else(|| AppError::BadRequest(format!("unknown provider: {name}")))?,
        None => ProviderKind::Google,
    };
    let db_provider = DbProvider::from(kind);
    let provider_name = kind.as_str();

    tracing::warn!(
        subject = %request.subject,
        provider = provider_name,
        "DEV LOGIN: forging a session without any provider verification"
    );

    let mut tx = state.pool.begin().await?;

    let user_id = match identities::find_by_provider_subject(
        &mut *tx,
        db_provider,
        &request.subject,
    )
    .await?
    {
        Some(identity) => identity.user_id,
        None => {
            let user = users::create(&mut *tx, None, None).await?;
            identities::create(&mut *tx, user.id, db_provider, &request.subject).await?;
            user.id
        }
    };

    let token = session::create(&mut *tx, user_id, &state.config.auth.session).await?;
    tx.commit().await?;

    let jar = jar.add(crate::http::auth::session_cookie(
        &state.config.auth.session,
        &token,
    ));

    Ok((
        jar,
        Json(serde_json::json!({
            "user_id": user_id,
            "provider": kind.as_str(),
            "subject": request.subject,
        }))
        .into_response(),
    ))
}

#[cfg(test)]
#[path = "dev_tests.rs"]
mod tests;
