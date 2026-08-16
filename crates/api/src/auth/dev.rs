//! Forged sign-in, for development only.
//!
//! # This is an authentication bypass
//!
//! Everything in this module skips the provider entirely: no authorization
//! redirect, no `state`, no PKCE, no ID token, no signature. A caller names an
//! identity and receives a session for it. Anyone who can reach the endpoint
//! can become any account.
//!
//! It exists because obtaining credentials for four providers is a serialised,
//! human-gated process — Apple alone needs a paid developer account — and the
//! rest of the product cannot be built or tested while that is outstanding.
//!
//! ## Why it cannot reach production
//!
//! The whole module, and the route that reaches it, are behind
//! `#[cfg(debug_assertions)]`. Cargo disables `debug_assertions` in the release
//! profile, and the container image builds `--release`, so **this code is not
//! in the shipped binary**. Not disabled in it — absent from it.
//!
//! That is the guarantee worth relying on. The `auth.dev_login` flag and the
//! startup warning are second and third lines for the case where someone runs a
//! debug build somewhere it was not expected.
//!
//! ## On the identifier
//!
//! The caller supplies a `subject`, and an email-shaped string is the natural
//! thing to type. It is stored as the provider subject — an opaque identifier
//! that merely looks like an email — because this service stores no email
//! address anywhere (D3, and phase 3's resolved question). Nothing parses it,
//! nothing mails it, and it never appears in a response.

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
    /// Optional display name for a freshly created account. Ignored when the
    /// account already exists, matching the real flow: a returning sign-in does
    /// not overwrite the user's name with whatever the provider currently says.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Signs in as `subject`, creating the account on first use.
///
/// Mirrors the real callback's identity mapping deliberately: same
/// `(provider, subject)` lookup, same find-or-create in one transaction, same
/// session issuance. A dev login that took a different path would let the real
/// path rot untested behind it.
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

    tracing::warn!(
        subject = %request.subject,
        provider = kind.as_str(),
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
            let user = users::create(&mut *tx, request.display_name.as_deref(), None).await?;
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
