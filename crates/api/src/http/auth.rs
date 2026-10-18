//! Authentication endpoints.
//!
//! ```text
//! GET  /auth/{provider}/start      begin sign-in
//! GET  /auth/{provider}/callback   OIDC / OAuth2 redirect callback
//! POST /auth/apple/callback        Apple form-POSTs instead
//! POST /auth/link/{provider}       add a provider to the signed-in account
//! POST /auth/logout                destroy the session server-side
//! GET  /api/me                     current user and linked providers
//! ```
//!
//! Callbacks consume browser-bound attempts before exchanging authorization
//! codes. Verification failures share a generic response and log their cause.
//! Return paths are validated before storage and redirect. Identity matching
//! uses provider and subject; linking uses the account recorded at flow start.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::auth::attempt::{self, BindingValue, PendingAttempt};
use crate::auth::provider::VerifiedIdentity;
use crate::auth::session::{self, SessionToken};
use crate::auth::sign_in::{GENERIC_FAILURE, SignInIdentity, sign_in};
use crate::auth::{AuthError, CurrentUser, ProviderKind};
use crate::config::SessionConfig;
use crate::db::repo::identities::{self, Provider as DbProvider};
use crate::db::repo::users;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Authorization attempts expire after this duration.
const ATTEMPT_TTL: Duration = Duration::minutes(10);

/// Message for a provider-declined flow, before account or identity writes.
const CANCELED: &str = "Sign-in was canceled, so nothing was created.";

/// Conflict message when an identity belongs to another account.
const ALREADY_LINKED: &str = "That account is already linked to a different PinVoyager account. \
     To bring it to this account, sign in to the other one first and unlink it there. \
     Nothing has been changed on either account.";

/// Where a completed sign-in lands when the caller asked for nowhere in
/// particular. The frontend decides what lives at the root; the flow calls it
/// Discover.
const SIGNED_IN_DESTINATION: &str = "/";

/// Linking starts in Settings → Sign-in methods, so it ends there too.
const LINKED_DESTINATION: &str = "/settings";

/// Allowed return paths. Entries match exactly or with a following slash.
const ALLOWED_REDIRECTS: &[&str] = &[
    "/",
    "/discover",
    "/settings",
    "/upload",
    "/voyages",
    "/admin",
];

/// Maximum number of bytes accepted in a return destination.
const MAX_REDIRECT_LEN: usize = 512;

/// Registers sign-in, linking, logout, and current-user routes. Both GET and
/// form-POST callbacks are supported for provider response modes.
pub fn routes() -> Router<AppState> {
    let router = Router::new()
        .route("/auth/{provider}/start", get(start))
        .route(
            "/auth/{provider}/callback",
            get(callback).post(form_callback),
        )
        .route("/auth/link/{provider}", post(link_start))
        .route("/auth/logout", post(logout))
        .route("/api/me", get(me).patch(super::profiles::update));

    // Not compiled into a release binary. `debug_assertions` is off in the
    // release profile and the container image builds `--release`, so this route
    // does not exist there — it is absent rather than disabled. The handler
    // additionally refuses unless `auth.dev_login` is set.
    #[cfg(debug_assertions)]
    let router = router.route("/auth/dev/login", post(crate::auth::dev::login));

    router
}

#[derive(Debug, Deserialize)]
struct StartQuery {
    /// Where to send the browser once sign-in completes. Validated before it
    /// is stored, and again before it is used.
    redirect_to: Option<String>,
}

/// Begins sign-in: create the attempt, then hand the browser to the provider.
async fn start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    jar: CookieJar,
    Query(query): Query<StartQuery>,
) -> AppResult<(CookieJar, Response)> {
    let kind = parse_provider(&provider)?;
    let (url, binding) = begin(&state, kind, None, query.redirect_to.as_deref()).await?;

    // 302 rather than 303: the browser is being sent onward to the provider,
    // not to the result of this request.
    let response = redirect(StatusCode::FOUND, url.as_str())?;
    Ok((jar.add(binding_cookie(&binding)), response))
}

/// Starts provider linking for the authenticated account. Records that account
/// and a browser-binding hash on the attempt before redirecting to the provider.
async fn link_start(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(provider): Path<String>,
    jar: CookieJar,
    Query(query): Query<StartQuery>,
) -> AppResult<(CookieJar, Response)> {
    let kind = parse_provider(&provider)?;
    // The user id is recorded on the attempt here, at the only moment it is
    // known to be authentic: inside a request carrying a valid session. The
    // callback reads it back rather than trusting anything the provider says.
    //
    // That alone is not enough: it fixes *which* account gets the identity but
    // not *whose* browser completes the flow. The binding cookie set below is
    // what stops an attacker starting a link on their own account and walking a
    // victim through the provider's consent screen.
    let (url, binding) = begin(
        &state,
        kind,
        Some(user.user_id),
        query.redirect_to.as_deref(),
    )
    .await?;

    // 303: the POST has been handled and the browser should GET the provider.
    let response = redirect(StatusCode::SEE_OTHER, url.as_str())?;
    Ok((jar.add(binding_cookie(&binding)), response))
}

/// Shared by both entry points: build the authorization URL and persist the
/// attempt that the callback will consume.
async fn begin(
    state: &AppState,
    kind: ProviderKind,
    link_user_id: Option<Uuid>,
    redirect_to: Option<&str>,
) -> AppResult<(Url, BindingValue)> {
    let provider = state.providers.get(kind).ok_or(AppError::NotFound)?;
    let authorization = provider.authorize().map_err(auth_failed)?;

    // Minted here and returned so the caller can set it as a cookie. Only the
    // hash is stored, so a database reader cannot complete someone's in-flight
    // authorization.
    let binding = BindingValue::generate();

    let attempt = PendingAttempt {
        state: authorization.state,
        provider: kind,
        nonce: authorization.nonce,
        pkce_verifier: authorization.pkce_verifier,
        link_user_id,
        // Validated *before* storage, so an unchecked value never reaches the
        // database in the first place and a later reader cannot inherit the
        // problem by trusting the column.
        redirect_to: redirect_to.and_then(safe_redirect),
        expires_at: Utc::now() + ATTEMPT_TTL,
        binding_hash: binding.hash(),
    };

    attempt::create(&state.pool, &attempt).await?;

    Ok((authorization.url, binding))
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Everything a callback carries, whichever way it arrived.
struct Callback<'a> {
    code: Option<&'a str>,
    state: Option<&'a str>,
    error: Option<&'a str>,
}

/// The ordinary redirect callback: Google and Microsoft.
async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Ok(kind) = parse_provider(&provider) else {
        return AppError::NotFound.into_response();
    };

    let outcome = complete(
        &state,
        kind,
        jar.clone(),
        Callback {
            code: query.code.as_deref(),
            state: query.state.as_deref(),
            error: query.error.as_deref(),
        },
    )
    .await;

    to_browser(outcome, jar, kind, &headers)
}

/// Apple's form-POST callback. Only authorization code, state, and error are
/// consumed; account display names are chosen inside PinVoyager.
async fn form_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    jar: CookieJar,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Ok(kind) = parse_provider(&provider) else {
        return AppError::NotFound.into_response();
    };
    let fields: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();

    let outcome = complete(
        &state,
        kind,
        jar.clone(),
        Callback {
            code: field(&fields, "code"),
            state: field(&fields, "state"),
            error: field(&fields, "error"),
        },
    )
    .await;

    to_browser(outcome, jar, kind, &headers)
}

/// Redirects callback outcomes to the frontend using 303 responses.
/// Preserves the session cookie on success and clears the binding cookie on
/// failure. An unconfigured provider returns 404 without redirecting.
fn to_browser(
    outcome: AppResult<(CookieJar, Response)>,
    jar: CookieJar,
    kind: ProviderKind,
    headers: &HeaderMap,
) -> Response {
    let err = match outcome {
        // The jar from `complete`, not the one passed in — on success it
        // carries the session cookie that was just issued.
        Ok(ok) => return ok.into_response(),
        Err(AppError::NotFound) => {
            return (jar.add(binding_cookie_removal()), AppError::NotFound).into_response();
        }
        Err(err) => err,
    };
    // `complete` returns an error without its updated jar. Clear the browser
    // binding here as well so rejected callbacks do not leave a stale cookie.
    let jar = jar.add(binding_cookie_removal());

    // Map provider decline, linking conflict, and other failures to frontend statuses.
    let status = match &err {
        AppError::BadRequest(message) if message == CANCELED => "canceled",
        AppError::Conflict(_) => "conflict",
        _ => "failed",
    };

    // Set on the request by the request-id middleware. It is the reference the
    // failure screen asks the user to quote, and it ties their report to the
    // log line this failure already produced.
    // Filtered rather than percent-encoded. The generator produces a UUID, so
    // everything legitimate survives this — but the header is attacker-supplied
    // (the request-id layer forwards a client's value if one is sent), and
    // interpolating an unfiltered string into a Location header is how response
    // splitting happens.
    let request_id: String = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();

    let provider_name = kind.as_str();
    let mut target = format!("/auth/callback?status={status}&provider={provider_name}");
    if !request_id.is_empty() {
        target.push_str("&ref=");
        target.push_str(&request_id);
    }

    tracing::warn!(
        provider = provider_name,
        status,
        "sign-in failed; redirecting to the callback screen"
    );

    // The jar still travels, because a failed callback has already had the
    // browser-binding cookie cleared and that removal must reach the browser.
    let response = redirect(StatusCode::SEE_OTHER, &target)
        .expect("callback targets contain only fixed paths and filtered ASCII values");
    (jar, response).into_response()
}

/// Consumes the attempt, checks browser binding and provider, then verifies
/// the authorization code. Signs in or links the verified identity. Missing,
/// expired, consumed, or mismatched attempts fail before account writes.
async fn complete(
    state: &AppState,
    kind: ProviderKind,
    jar: CookieJar,
    callback: Callback<'_>,
) -> AppResult<(CookieJar, Response)> {
    let Some(csrf_state) = callback.state else {
        return Err(rejected("callback carried no state"));
    };

    // `Ok(None)` is unknown, already-used, or expired, and the caller is told
    // the same thing for all three. Even the log cannot tell them apart — the
    // delete is atomic and deliberately does not report why it matched nothing.
    let Some(attempt) = attempt::consume(&state.pool, csrf_state).await? else {
        return Err(rejected("no pending attempt for this state"));
    };

    // The callback must arrive in the browser that started the flow.
    //
    // This is what closes link CSRF: an attacker can hand a victim a URL
    // carrying the attacker's `state`, but cannot put the matching cookie in
    // the victim's browser. Checked for sign-in as well as linking — a forged
    // sign-in completion is a lesser problem but not a desirable one, and one
    // code path is easier to keep correct than two.
    let presented = jar
        .get(BINDING_COOKIE)
        .map(|cookie| BindingValue::from_cookie_value(cookie.value()));

    // Every path below returns through this jar, so the binding cookie is
    // cleared whether the flow succeeds or fails. Read it before adding the
    // removal cookie: CookieJar::get observes the jar's current value, and the
    // removal value is empty.
    let jar = jar.add(binding_cookie_removal());

    match presented {
        Some(value) if value.matches(&attempt.binding_hash) => {}
        Some(_) => return Err(rejected("binding cookie did not match the attempt")),
        None => return Err(rejected("callback carried no binding cookie")),
    }

    // The state belongs to a flow this service started, but not necessarily to
    // *this* provider's flow. Without this check a state issued for one
    // provider could be redeemed at another, and the nonce and PKCE verifier
    // would be checked against the wrong party.
    if attempt.provider != kind {
        return Err(rejected("state was issued for a different provider"));
    }

    if let Some(error) = callback.error {
        // Provider decline returns before account or identity changes.
        return Err(if error == "access_denied" {
            tracing::info!(provider = kind.as_str(), "user declined at the provider");
            AppError::BadRequest(CANCELED.to_owned())
        } else {
            rejected("provider returned an error")
        });
    }

    let Some(code) = callback.code else {
        return Err(rejected("callback carried no code"));
    };

    let provider = state.providers.get(kind).ok_or(AppError::NotFound)?;

    // Verification lives behind this call: JWKS signature, `iss`, `aud`, `exp`
    // and `nonce` for all three OIDC providers. Whatever
    // fails inside it reaches the client as the same sentence.
    let identity = provider
        .exchange(code, &attempt.pkce_verifier, &attempt.nonce)
        .await
        .map_err(auth_failed)?;

    // A provider implementation that returned someone else's `ProviderKind`
    // would write a subject under the wrong provider, and the unique index
    // would then let it collide with a real identity. Cheaper to refuse.
    if identity.provider != kind {
        return Err(rejected(
            "provider returned an identity for another provider",
        ));
    }

    match attempt.link_user_id {
        // T9. The account is the one that *started* the flow. Using the account
        // implied by the returned identity would let anyone who can complete a
        // provider round trip attach it to whichever account they liked.
        Some(user_id) => {
            link(state, user_id, &identity).await?;

            // No session is issued and no cookie is set: the caller is already
            // signed in as `user_id`, and session A stays exactly as it was.
            let to = destination(attempt.redirect_to.as_deref(), LINKED_DESTINATION);
            Ok((jar, redirect(StatusCode::SEE_OTHER, &to)?))
        }
        None => {
            let (_, token) = sign_in(state, SignInIdentity::Verified(&identity)).await?;
            let jar = jar.add(session_cookie(&state.config.auth.session, &token));

            let to = destination(attempt.redirect_to.as_deref(), SIGNED_IN_DESTINATION);
            Ok((jar, redirect(StatusCode::SEE_OTHER, &to)?))
        }
    }
}

/// Links the verified provider identity to the supplied account.
///
/// An identity linked to another account returns a conflict before any changes
/// commit. Linking an identity already on this account succeeds. No session is issued.
async fn link(state: &AppState, user_id: Uuid, identity: &VerifiedIdentity) -> AppResult<()> {
    let provider = DbProvider::from(identity.provider);

    let mut tx = state.pool.begin().await?;
    users::require_active(&mut tx, user_id).await?;

    if let Some(existing) =
        identities::find_by_provider_subject(&mut *tx, provider, &identity.subject).await?
    {
        if existing.user_id != user_id {
            let provider_name = identity.provider.as_str();
            tracing::warn!(
                provider = provider_name,
                linking_user_id = %user_id,
                owning_user_id = %existing.user_id,
                "refused to link an identity owned by another account"
            );
            // The subject is deliberately absent from that line: it is the
            // provider's identifier for a person, and this is a log, not a
            // record we need.
            return Err(AppError::Conflict(ALREADY_LINKED.to_owned()));
        }

        // An identity already linked to this account succeeds without another insert.
        return Ok(());
    }

    identities::create(&mut *tx, user_id, provider, &identity.subject)
        .await
        .map_err(link_identity_error)?;

    tx.commit().await?;

    Ok(())
}

fn link_identity_error(err: sqlx::Error) -> AppError {
    if unique_violation(&err) {
        // Another flow claimed this identity between the SELECT and the INSERT.
        AppError::Conflict(ALREADY_LINKED.to_owned())
    } else {
        err.into()
    }
}

/// Revokes the presented session and clears its cookie, returning 204.
/// Missing or expired sessions are accepted. A database failure is returned
/// before cookie removal.
async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<(CookieJar, StatusCode)> {
    let name = &state.config.auth.session.cookie_name;

    if let Some(cookie) = jar.get(name) {
        session::revoke(
            &state.pool,
            &SessionToken::from_cookie_value(cookie.value()),
        )
        .await?;
    }

    // The removal has to carry the same `Path` as the cookie that was set, or
    // the browser treats it as a different cookie and keeps the original.
    let jar = jar.remove(Cookie::build((name.clone(), "")).path("/").build());

    Ok((jar, StatusCode::NO_CONTENT))
}

#[derive(Debug, Serialize)]
struct MeResponse {
    user: MeUser,
    identities: Vec<LinkedProvider>,
}

#[derive(Debug, Serialize)]
struct MeUser {
    id: Uuid,
    is_site_admin: bool,
    display_name: Option<String>,
    avatar_url: Option<String>,
    created_at: DateTime<Utc>,
}

/// What Settings → Sign-in methods draws. No subject: it is the provider's
/// identifier for a person and the screen has no use for it.
#[derive(Debug, Serialize)]
struct LinkedProvider {
    provider: String,
    linked_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

/// Returns the current account and linked providers. Missing accounts return 401.
async fn me(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<MeResponse>> {
    // A missing account is treated as an unauthenticated session.
    let account = users::find_by_id(&state.pool, user.user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Written out here rather than in `db::repo::identities` for the same
    // reason as the last-seen update: one call site, one query.
    let identities = sqlx::query!(
        "SELECT provider, created_at, last_seen_at
         FROM identities
         WHERE user_id = $1
         ORDER BY created_at",
        user.user_id,
    )
    .fetch_all(&state.pool)
    .await?;

    let is_site_admin = sqlx::query_scalar("SELECT is_site_admin FROM users WHERE id=$1")
        .bind(user.user_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(Json(MeResponse {
        user: MeUser {
            id: account.id,
            is_site_admin,
            display_name: account.display_name,
            avatar_url: account.avatar_url,
            created_at: account.created_at,
        },
        identities: identities
            .into_iter()
            .map(|row| LinkedProvider {
                provider: row.provider,
                linked_at: row.created_at,
                last_seen_at: row.last_seen_at,
            })
            .collect(),
    }))
}

/// Builds the session cookie with HttpOnly, SameSite=Lax, Path=/, and the
/// configured Secure flag. Max-Age uses the absolute session lifetime; server
/// lookup separately enforces inactivity expiry.
pub(crate) fn session_cookie(config: &SessionConfig, token: &SessionToken) -> Cookie<'static> {
    Cookie::build((config.cookie_name.clone(), token.expose().to_owned()))
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(SameSite::Lax)
        .path("/")
        // Keep the cookie until the absolute session deadline. Server lookup
        // can reject it earlier after the configured period of inactivity.
        .max_age(cookie::time::Duration::seconds(clamp_secs(
            config.absolute_timeout_secs,
        )))
        .build()
}

/// Name of the browser-binding cookie. Distinct from the session cookie, and
/// deliberately not configurable — nothing outside this module should be
/// reasoning about it.
const BINDING_COOKIE: &str = "pv_auth_binding";

/// Builds the browser-binding cookie: HttpOnly, Secure, SameSite=None,
/// Path=/, and Max-Age matching the attempt lifetime. SameSite=None permits
/// Apple's cross-site form callback to carry the binding.
fn binding_cookie(value: &BindingValue) -> Cookie<'static> {
    Cookie::build((BINDING_COOKIE, value.expose().to_owned()))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::None)
        .path("/")
        .max_age(cookie::time::Duration::seconds(
            ATTEMPT_TTL.num_seconds().max(0),
        ))
        .build()
}

/// The removal form of the binding cookie.
///
/// Attributes must match what was set or the browser keeps the original: a
/// removal is matched on name, path and domain, not on value.
fn binding_cookie_removal() -> Cookie<'static> {
    Cookie::build((BINDING_COOKIE, ""))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::None)
        .path("/")
        .max_age(cookie::time::Duration::seconds(0))
        .build()
}

/// Converts cookie lifetime seconds to `i64`, saturating at `i64::MAX`.
fn clamp_secs(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Returns a permitted local destination unchanged, or `None` for a missing,
/// empty, overlong, non-ASCII, unsafe, or unrecognized path. Query strings and
/// fragments are retained; only the path is matched against the allowlist.
fn safe_redirect(candidate: &str) -> Option<String> {
    if candidate.len() > MAX_REDIRECT_LEN || !candidate.starts_with('/') {
        return None;
    }

    // `//evil.example` and `/\evil.example` are protocol-relative URLs: a
    // browser reads both as another origin despite the leading slash, and the
    // second only because it folds backslashes to forward slashes while a
    // string comparison does not. Backslash is refused anywhere in the value
    // for that reason.
    if candidate.starts_with("//") || candidate.contains('\\') {
        return None;
    }

    // Printable ASCII only. This excludes the CR and LF that would otherwise be
    // a response-splitting attempt in the `Location` header, and non-ASCII
    // bytes, which have to be percent-encoded to be legal in a URL anyway.
    if candidate.bytes().any(|b| !(0x20..0x7f).contains(&b)) {
        return None;
    }

    // Only the path is matched. A query string or fragment is the frontend's
    // business; the origin is ours, and the origin is settled by the checks
    // above plus the allowlist below.
    let path = candidate
        .split(['?', '#'])
        .next()
        .unwrap_or(SIGNED_IN_DESTINATION);

    let allowed = ALLOWED_REDIRECTS.iter().any(|prefix| {
        path == *prefix
            // A path *beneath* an entry, so `/voyages` covers `/voyages/{id}`.
            // The explicit separator check is what stops `/settings-evil` from
            // passing on the strength of `/settings`.
            || (path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/'))
    });

    allowed.then(|| candidate.to_owned())
}

/// Revalidates a stored return path and falls back to the default destination.
fn destination(stored: Option<&str>, fallback: &str) -> String {
    stored
        .and_then(safe_redirect)
        .unwrap_or_else(|| fallback.to_owned())
}

/// Resolves a configured provider; unknown and disabled providers return 404.
fn parse_provider(name: &str) -> AppResult<ProviderKind> {
    ProviderKind::parse(name).ok_or(AppError::NotFound)
}

/// One client-visible outcome for every way a flow can fail, and one log line
/// that says which it was.
///
/// The log line carries the request id from the surrounding span — the same id
/// the response carries in `x-request-id` and the failure screen shows as a
/// reference — so "we could not complete sign-in" is still diagnosable by
/// anyone with the log, and by nobody with only the response.
fn rejected(reason: &'static str) -> AppError {
    tracing::warn!(reason, "sign-in rejected");
    AppError::BadRequest(GENERIC_FAILURE.to_owned())
}

/// The same, for a failure that arrived as an [`AuthError`].
fn auth_failed(err: AuthError) -> AppError {
    match err {
        // Not a failure of this attempt: the provider genuinely is not
        // available here, and 404 is what every other unconfigured-provider
        // path returns.
        AuthError::ProviderNotConfigured => AppError::NotFound,
        other => {
            tracing::warn!(error = %other, "sign-in rejected");
            AppError::BadRequest(GENERIC_FAILURE.to_owned())
        }
    }
}

/// Returns whether a database error reports the provider/subject uniqueness
/// constraint, indicating that an identity is already claimed.
fn unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Builds a redirect by hand because `axum::response::Redirect` offers 303, 307
/// and 308, and the start endpoint wants 302.
fn redirect(status: StatusCode, location: &str) -> AppResult<Response> {
    let location =
        HeaderValue::from_str(location).map_err(|err| AppError::Internal(Box::new(err)))?;

    Ok((status, [(header::LOCATION, location)]).into_response())
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
