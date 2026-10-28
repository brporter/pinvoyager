//! Session extractors for required and optional authentication.
//!
//! `CurrentUser` rejects missing or expired sessions. `MaybeUser` allows
//! anonymous requests. Both update activity when a valid session is found.

use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, header, request::Parts};
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

/// Parses one native bearer credential. Never falls back to a browser cookie.
pub(crate) fn native_token(
    headers: &HeaderMap,
    cookie_name: &str,
) -> Result<Option<SessionToken>, AppError> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1
        || CookieJar::from_headers(headers).get(cookie_name).is_some()
    {
        return Err(AppError::Unauthorized);
    }
    let value = value.to_str().map_err(|_| AppError::Unauthorized)?;
    let (scheme, token) = value.split_once(' ').ok_or(AppError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.len() > 1024
        || token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || !byte.is_ascii_graphic())
    {
        return Err(AppError::Unauthorized);
    }
    Ok(Some(SessionToken::from_cookie_value(token)))
}

/// Checks the credential's transport before touching session activity.
async fn resolve(parts: &Parts, state: &AppState) -> Result<Option<CurrentUser>, AppError> {
    let config = &state.config.auth.session;
    let bearer = native_token(&parts.headers, &config.cookie_name)?;
    let native = bearer.is_some();
    let jar = CookieJar::from_headers(&parts.headers);
    let token = bearer.or_else(|| {
        jar.get(&config.cookie_name)
            .map(|cookie| SessionToken::from_cookie_value(cookie.value()))
    });
    let Some(token) = token else {
        return Ok(None);
    };
    let is_native: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM native_sessions WHERE token_hash=$1)")
            .bind(token.hash())
            .fetch_one(&state.pool)
            .await?;
    if native != is_native {
        return if native {
            Err(AppError::Unauthorized)
        } else {
            Ok(None)
        };
    }
    let Some(session) = session::lookup_and_touch(&state.pool, &token, config).await? else {
        return if native {
            Err(AppError::Unauthorized)
        } else {
            Ok(None)
        };
    };

    let active: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND NOT suspended)")
            .bind(session.user_id)
            .fetch_one(&state.pool)
            .await?;
    if !active {
        return if native {
            Err(AppError::Unauthorized)
        } else {
            Ok(None)
        };
    }

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
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = match resolve(parts, state).await {
            Ok(user) => user,
            // Log database failures and continue as anonymous for optional authentication.
            Err(AppError::Database(err)) if !parts.headers.contains_key(header::AUTHORIZATION) => {
                tracing::warn!(error = %err, "session lookup failed; treating request as anonymous");
                None
            }
            Err(err) => return Err(err),
        };

        Ok(MaybeUser(user))
    }
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
