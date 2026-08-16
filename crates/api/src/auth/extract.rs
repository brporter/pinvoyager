//! Authentication extractors.
//!
//! CONTRACT — types are fixed; the `FromRequestParts` implementations are owned
//! by the session agent.
//!
//! The point of these is that authentication becomes a **type-level**
//! requirement. A handler that takes [`CurrentUser`] cannot run without one, so
//! there is no `if !authenticated { return }` to forget. Phase 5's ownership
//! checks build on this.

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
/// anonymous callers — which is most of the read surface, since every voyage is
/// public (D6).
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<CurrentUser>);

/// The shared half of both extractors.
///
/// Both go through `lookup_and_touch` rather than a plain read, because the
/// sliding expiry only slides if every authenticated request touches the row —
/// including the ones that reach an endpoint through [`MaybeUser`]. An
/// optional-auth endpoint is still activity.
///
/// A database failure is distinct from "not signed in" and stays a `sqlx::Error`
/// here so that each extractor can decide what to do with it; folding it into
/// `None` would turn an outage into a wave of silent sign-outs.
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
        // probing client that a token was *recognised but expired* confirms the
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
            // Infallible by contract, so a database failure degrades to
            // anonymous rather than failing the request. That is the right
            // trade for a read surface that is public anyway (D6), but it is
            // invisible unless it is logged — hence the warning.
            Err(err) => {
                tracing::warn!(error = %err, "session lookup failed; treating request as anonymous");
                None
            }
        };

        Ok(MaybeUser(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::http::Request;
    use secrecy::SecretString;
    use sqlx::PgPool;

    use crate::auth::ProviderRegistry;
    use crate::config::{AuthConfig, Config, DatabaseConfig};

    fn app_state(pool: PgPool) -> AppState {
        let config = Config {
            server: crate::config::ServerConfig::default(),
            database: DatabaseConfig {
                url: SecretString::from("postgres://unused".to_owned()),
                max_connections: 1,
                min_connections: 1,
                acquire_timeout_secs: 5,
            },
            telemetry: crate::config::TelemetryConfig::default(),
            auth: AuthConfig {
                base_url: "https://example.test".parse().expect("valid base url"),
                session: crate::config::SessionConfig::default(),
                providers: crate::config::ProvidersConfig::default(),
            },
        };

        AppState::new(pool, config, ProviderRegistry::empty())
    }

    fn parts(cookie: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri("/api/me");
        if let Some(cookie) = cookie {
            builder = builder.header(axum::http::header::COOKIE, cookie);
        }

        builder.body(()).expect("valid request").into_parts().0
    }

    #[sqlx::test]
    async fn current_user_resolves_from_the_session_cookie(pool: PgPool) {
        let state = app_state(pool.clone());
        let user = crate::db::repo::users::create(&pool, Some("Ada"), None)
            .await
            .expect("create user");

        let token = session::create(&pool, user.id, &state.config.auth.session)
            .await
            .expect("create session");

        let cookie = format!("pv_session={}", token.expose());
        let mut parts = parts(Some(&cookie));

        let current = CurrentUser::from_request_parts(&mut parts, &state)
            .await
            .expect("authenticated");

        assert_eq!(current.user_id, user.id);
        assert_eq!(current.session.user_id, user.id);
    }

    #[sqlx::test]
    async fn extraction_slides_the_session_forward(pool: PgPool) {
        let state = app_state(pool.clone());
        let user = crate::db::repo::users::create(&pool, None, None)
            .await
            .expect("create user");

        let token = session::create(&pool, user.id, &state.config.auth.session)
            .await
            .expect("create session");

        sqlx::query("UPDATE sessions SET last_seen_at = now() - interval '1 hour'")
            .execute(&pool)
            .await
            .expect("age session");

        let cookie = format!("pv_session={}", token.expose());
        let mut parts = parts(Some(&cookie));

        // Optional auth is still activity, so this must slide the expiry too.
        let MaybeUser(user) = MaybeUser::from_request_parts(&mut parts, &state)
            .await
            .expect("infallible");
        assert!(user.is_some());

        let last_seen: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT last_seen_at FROM sessions")
                .fetch_one(&pool)
                .await
                .expect("read session");

        assert!(last_seen > chrono::Utc::now() - chrono::Duration::minutes(1));
    }

    #[sqlx::test]
    async fn current_user_is_unauthorized_without_a_cookie(pool: PgPool) {
        let state = app_state(pool);
        let mut parts = parts(None);

        let err = CurrentUser::from_request_parts(&mut parts, &state)
            .await
            .expect_err("no session");

        assert!(matches!(err, AppError::Unauthorized));
    }

    #[sqlx::test]
    async fn current_user_is_unauthorized_when_the_session_was_revoked(pool: PgPool) {
        let state = app_state(pool.clone());
        let user = crate::db::repo::users::create(&pool, None, None)
            .await
            .expect("create user");

        let token = session::create(&pool, user.id, &state.config.auth.session)
            .await
            .expect("create session");
        session::revoke(&pool, &token).await.expect("revoke");

        let cookie = format!("pv_session={}", token.expose());
        let mut parts = parts(Some(&cookie));

        let err = CurrentUser::from_request_parts(&mut parts, &state)
            .await
            .expect_err("revoked session");

        assert!(matches!(err, AppError::Unauthorized));
    }

    #[sqlx::test]
    async fn maybe_user_is_none_for_an_anonymous_caller(pool: PgPool) {
        let state = app_state(pool);
        let mut parts = parts(Some("some_other_cookie=value"));

        let MaybeUser(user) = MaybeUser::from_request_parts(&mut parts, &state)
            .await
            .expect("infallible");

        assert!(user.is_none());
    }
}
