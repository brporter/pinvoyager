//! Session extractors for required and optional authentication.
//!
//! `CurrentUser` rejects missing or expired sessions. `MaybeUser` allows
//! anonymous requests. Both update activity when a valid session is found.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;
use uuid::Uuid;

use crate::auth::session::{self, Session, SessionToken};
use crate::error::AppError;
use crate::state::AppState;

/// A signed-in user. Extraction fails with 401 when absent or expired.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user_id: Uuid,
    pub session: Session,
}

/// A user if there is one. For endpoints that serve both signed-in and
/// anonymous callers — which is most of the read surface, since a published voyage is
/// public (D6).
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<CurrentUser>);

/// Reads the session cookie and looks up its token, updating session activity.
/// Returns `Ok(None)` for a missing cookie or invalid session and propagates
/// database errors to the extractor.
async fn resolve(parts: &Parts, state: &AppState) -> Result<Option<CurrentUser>, sqlx::Error> {
    let config = &state.config.auth.session;

    let jar = CookieJar::from_headers(&parts.headers);
    let Some(cookie) = jar.get(&config.cookie_name) else {
        return Ok(None);
    };

    let token = SessionToken::from_cookie_value(cookie.value());
    let Some(session) = session::lookup_and_touch(&state.pool, &token, config).await? else {
        return Ok(None);
    };

    Ok(Some(CurrentUser {
        user_id: session.user_id,
        session,
    }))
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // No cookie, an unknown token, and an expired session are all 401. The
        // caller learns that it must sign in and nothing else — telling a
        // probing client that a token was *recognized but expired* confirms the
        // token was real.
        resolve(parts, state).await?.ok_or(AppError::Unauthorized)
    }
}

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = match resolve(parts, state).await {
            Ok(user) => user,
            // Log database failures and continue as anonymous for optional authentication.
            Err(err) => {
                tracing::warn!(error = %err, "session lookup failed; treating request as anonymous");
                None
            }
        };

        Ok(MaybeUser(user))
    }
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
