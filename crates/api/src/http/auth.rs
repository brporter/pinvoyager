//! Authentication endpoints.
//!
//! CONTRACT — `routes()` is fixed; the handlers are owned by the endpoints
//! agent.
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
//! Four properties this module is responsible for. Each of them is invisible
//! while it holds and expensive once it does not:
//!
//! **Every failure looks the same.** A rejected `state`, a replayed callback,
//! an expired attempt, a bad signature and a nonce mismatch all produce one
//! response. Distinguishing them for a user distinguishes them for someone
//! probing the endpoint; the real cause goes to the log with the request id,
//! which is the reference the design's failure screen tells the user to quote.
//!
//! **`redirect_to` is validated before it is stored.** A sign-in link is one a
//! user is *expected* to click, so an unchecked destination turns this service
//! into a credible springboard for phishing. See [`safe_redirect`].
//!
//! **Linking uses the account that started the flow**, taken from the pending
//! attempt, never from the identity that comes back. Proving control of B and
//! claiming A is precisely the direction D3 exists to close.
//!
//! **Nothing is matched on email**, because nothing here has an email to match
//! on: [`VerifiedIdentity`] has no such field and neither does the schema
//! (D3). Identity resolution is `(provider, subject)` and nothing else.

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
use crate::auth::{AuthError, CurrentUser, ProviderKind};
use crate::config::SessionConfig;
use crate::db::repo::identities::{self, Provider as DbProvider};
use crate::db::repo::users;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// How long a started sign-in stays completable.
///
/// Minutes, not hours: an attempt is a window in which a callback carrying the
/// right `state` is accepted, and the only thing a longer window buys is a
/// larger set of live states for an attacker to guess at. Ten minutes is
/// comfortably longer than a human consent screen and shorter than a coffee
/// break, and an attempt that expires produces the "starting again normally
/// works" screen, which is true.
const ATTEMPT_TTL: Duration = Duration::minutes(10);

/// The one thing a caller is told about any verification failure.
///
/// Deliberately says nothing about which check failed and offers the action
/// that usually works, matching the designed screen.
const GENERIC_FAILURE: &str = "We could not complete sign-in. Starting again normally works.";

/// Declining at the provider is a normal choice, not an error, and the design
/// gives it a neutral screen. "Nothing was created" is the reassurance that
/// matters, and it is true: nothing is written before the exchange.
const CANCELLED: &str = "Sign-in was cancelled, so nothing was created.";

/// The provider-conflict copy, from `design/screens/auth-states.html`. The last
/// sentence is a promise about server state, not a soothing noise — see
/// [`link`], which is written so that it cannot become false.
const ALREADY_LINKED: &str = "That account is already linked to a different PinVoyager account. \
     To bring it to this account, sign in to the other one first and unlink it there. \
     Nothing has been changed on either account.";

/// Where a completed sign-in lands when the caller asked for nowhere in
/// particular. The frontend decides what lives at the root; the flow calls it
/// Discover.
const SIGNED_IN_DESTINATION: &str = "/";

/// Linking starts in Settings → Sign-in methods, so it ends there too.
const LINKED_DESTINATION: &str = "/settings";

/// The allowlist behind [`safe_redirect`].
///
/// An entry matches its own path exactly, or any path beneath it. It is a list
/// of prefixes rather than a pattern because a pattern is a thing to get subtly
/// wrong once and never look at again, whereas an entry added here is a
/// deliberate act.
const ALLOWED_REDIRECTS: &[&str] = &["/", "/discover", "/settings", "/upload", "/voyages"];

/// Upper bound on a stored destination. Nothing legitimate is close to this;
/// it exists so a caller cannot use the attempts table as free storage.
const MAX_REDIRECT_LEN: usize = 512;

/// All authentication routes.
///
/// Apple needs a POST callback as well as the GET one because it uses
/// `response_mode=form_post` — its redirect is a form submission, not a query
/// string. Registering only the GET produces a 405 that looks like a routing
/// bug and is actually a protocol difference.
///
/// The POST is registered on the same parameterised path rather than on a
/// literal `/auth/apple/callback`, which serves that exact URL while keeping
/// one route per concept. A POST callback from another provider would be
/// handled identically and would still have to pass every check, so there is
/// nothing to gain by refusing it by name.
pub fn routes() -> Router<AppState> {
    let router = Router::new()
        .route("/auth/{provider}/start", get(start))
        .route(
            "/auth/{provider}/callback",
            get(callback).post(form_callback),
        )
        .route("/auth/link/{provider}", post(link_start))
        .route("/auth/logout", post(logout))
        .route("/api/me", get(me));

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

/// Begins a link flow for the **already signed-in** user (T9).
///
/// Requires [`CurrentUser`], so there is no path through this handler for an
/// anonymous caller — the requirement is a type, not an `if`. POST rather than
/// GET because it starts a state change, and because a `SameSite=Lax` cookie is
/// not sent on a cross-site POST, which is what stops another site from
/// starting a link flow in a signed-in user's browser.
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
    /// `access_denied` when the user declined; anything else is a provider-side
    /// failure. Every field here is optional because these arrive from the
    /// provider's redirect and a missing one must be a rejection, not a 422
    /// that says which field was absent.
    error: Option<String>,
}

/// Everything a callback carries, whichever way it arrived.
struct Callback<'a> {
    code: Option<&'a str>,
    state: Option<&'a str>,
    error: Option<&'a str>,
    /// Apple's raw form body, passed through to the provider untouched.
    form_body: Option<&'a str>,
}

/// The ordinary redirect callback: Google, Microsoft, Facebook.
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
            form_body: None,
        },
    )
    .await;

    to_browser(outcome, jar, kind, &headers)
}

/// Apple's callback: `response_mode=form_post`, so a POST with a form body.
///
/// The body is taken as a string and handed to the provider whole. Apple's
/// first-authorization `user` payload is a JSON object *inside* this body and
/// arrives exactly once, ever (D1); re-serialising it here would mean this
/// handler deciding what parts of it survive, and anything it dropped would be
/// unrecoverable. Extracting `code` and `state` is this handler's only claim on
/// the body.
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
            form_body: Some(&body),
        },
    )
    .await;

    to_browser(outcome, jar, kind, &headers)
}

/// Turns a callback result into something a *browser* can render.
///
/// Both callbacks are top-level navigations: the provider redirects the user's
/// browser here, or POSTs a form to it. Whatever this returns is what fills the
/// window. A JSON error body — which is the right answer everywhere else in
/// this service — puts `{"code":"bad_request","message":"..."}` on screen as
/// plain text, and the designed failure screens in
/// `design/screens/auth-states.html` are never reached at all.
///
/// So a failure becomes a 303 to the frontend's `/auth/callback` screen,
/// carrying the outcome in the query string. The screen decides what to show;
/// this decides only which of the four outcomes it was.
///
/// `NotFound` is deliberately left as a 404. It means the provider is not
/// configured, which is not a sign-in that failed — it is a URL that does not
/// exist here, and redirecting to a "sign-in failed" screen would tell the user
/// a story about their attempt rather than about our configuration.
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
        Err(AppError::NotFound) => return AppError::NotFound.into_response(),
        Err(err) => err,
    };

    // The four states the screen draws. `cancelled` is matched on the message
    // constant rather than on a variant because declining at the provider is
    // the one outcome that is not an error — it is a choice — and it reaches
    // here as an ordinary `BadRequest`.
    let status = match &err {
        AppError::BadRequest(message) if message == CANCELLED => "cancelled",
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

    let mut target = format!("/auth/callback?status={status}&provider={}", kind.as_str());
    if !request_id.is_empty() {
        target.push_str("&ref=");
        target.push_str(&request_id);
    }

    tracing::warn!(
        provider = kind.as_str(),
        status,
        "sign-in failed; redirecting to the callback screen"
    );

    // The jar still travels, because a failed callback has already had the
    // browser-binding cookie cleared and that removal must reach the browser.
    match redirect(StatusCode::SEE_OTHER, &target) {
        Ok(response) => (jar, response).into_response(),
        Err(err) => err.into_response(),
    }
}

/// The callback, once the transport difference has been flattened away.
///
/// Order matters here. The attempt is consumed before anything else is looked
/// at, so a replay fails on its second use whatever else it carries, and so
/// that no branch below can be reached without a `state` this service issued.
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

    // Every path below returns through this jar, so the binding cookie is
    // cleared whether the flow succeeds or fails. Leaving it behind would let a
    // browser accumulate one per abandoned sign-in, and a stale value can only
    // ever cause a confusing mismatch later.
    let jar = jar.add(binding_cookie_removal());

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
        // Safe to answer honestly: reaching this line required a state this
        // service issued and has just consumed, so the answer cannot be used to
        // probe for anything. Nothing has been written, which is what the
        // neutral screen promises.
        return Err(if error == "access_denied" {
            tracing::info!(provider = kind.as_str(), "user declined at the provider");
            AppError::BadRequest(CANCELLED.to_owned())
        } else {
            rejected("provider returned an error")
        });
    }

    let Some(code) = callback.code else {
        return Err(rejected("callback carried no code"));
    };

    let provider = state.providers.get(kind).ok_or(AppError::NotFound)?;

    // Verification lives behind this call: JWKS signature, `iss`, `aud`, `exp`
    // and `nonce` for the OIDC three, a Graph lookup for Facebook. Whatever
    // fails inside it reaches the client as the same sentence.
    let identity = provider
        .exchange(
            code,
            &attempt.pkce_verifier,
            &attempt.nonce,
            callback.form_body,
        )
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
            let token = sign_in(state, &identity).await?;
            let jar = jar.add(session_cookie(&state.config.auth.session, &token));

            let to = destination(attempt.redirect_to.as_deref(), SIGNED_IN_DESTINATION);
            Ok((jar, redirect(StatusCode::SEE_OTHER, &to)?))
        }
    }
}

/// Resolves a verified identity to an account and issues a session (T7).
///
/// The lookup is `(provider, subject)` and nothing else. There is no email
/// branch here, and there is no email to write one with: `VerifiedIdentity` has
/// no such field and the schema has no such column (D3). Two providers can
/// return the same address for the same human while not every provider verifies
/// ownership, so a match on email is an account-takeover vector rather than a
/// convenience.
async fn sign_in(state: &AppState, identity: &VerifiedIdentity) -> AppResult<SessionToken> {
    let provider = DbProvider::from(identity.provider);

    // One transaction for the whole unit of work: on any failure below there is
    // no account without an identity, and no identity without a session to use
    // it. It is short and holds no connection across unrelated work (D8).
    let mut tx = state.pool.begin().await?;

    let user_id =
        match identities::find_by_provider_subject(&mut *tx, provider, &identity.subject).await? {
            Some(existing) => {
                // A returning sign-in updates last-seen and nothing else. In
                // particular the provider's current name and avatar do not
                // overwrite what the user has since set here — the provider is the
                // authority on who they are, not on how they look to us.
                //
                // Written out here rather than in `db::repo::identities` because it
                // serves this one call site; move it there when a second appears.
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
                let user = users::create(
                    &mut *tx,
                    identity.display_name.as_deref(),
                    identity.avatar_url.as_deref(),
                )
                .await?;

                identities::create(&mut *tx, user.id, provider, &identity.subject)
                    .await
                    .map_err(|err| {
                        if unique_violation(&err) {
                            // Two first sign-ins for the same subject raced and the
                            // unique index arbitrated. The loser retries and lands
                            // in the branch above; this is not a 500.
                            rejected("concurrent first sign-in for the same identity")
                        } else {
                            err.into()
                        }
                    })?;

                user.id
            }
        };

    let token = session::create(&mut *tx, user_id, &state.config.auth.session).await?;

    tx.commit().await?;

    Ok(token)
}

/// Attaches a verified identity to an existing account (T9).
///
/// The conflict path is why this is written the way it is. Before it, the only
/// statement issued is a `SELECT`; the refusal returns before any write and
/// drops the transaction, which rolls it back. The other account's rows —
/// including its identity's `last_seen_at` — are not touched, and no session is
/// created or altered on either side. "Nothing has been changed on either
/// account" is a promise the design prints on a screen, so it has to be a
/// property of this function rather than a hope.
async fn link(state: &AppState, user_id: Uuid, identity: &VerifiedIdentity) -> AppResult<()> {
    let provider = DbProvider::from(identity.provider);

    let mut tx = state.pool.begin().await?;

    if let Some(existing) =
        identities::find_by_provider_subject(&mut *tx, provider, &identity.subject).await?
    {
        if existing.user_id != user_id {
            tracing::warn!(
                provider = identity.provider.as_str(),
                linking_user_id = %user_id,
                owning_user_id = %existing.user_id,
                "refused to link an identity owned by another account"
            );
            // The subject is deliberately absent from that line: it is the
            // provider's identifier for a person, and this is a log, not a
            // record we need.
            return Err(AppError::Conflict(ALREADY_LINKED.to_owned()));
        }

        // Already linked to this same account. Linking twice is not an error and
        // must not become one: the user cannot tell from Settings whether a
        // half-finished flow completed, so the honest answer is "it is linked".
        return Ok(());
    }

    identities::create(&mut *tx, user_id, provider, &identity.subject)
        .await
        .map_err(|err| {
            if unique_violation(&err) {
                // Another flow claimed this identity between the SELECT and the
                // INSERT. The unique index is the real guard; the check above
                // exists to produce a better message, not to be the mechanism.
                AppError::Conflict(ALREADY_LINKED.to_owned())
            } else {
                err.into()
            }
        })?;

    tx.commit().await?;

    Ok(())
}

/// Signs the caller out (T10).
///
/// The row is deleted, not merely the cookie: a cleared cookie leaves a live
/// session that anyone still holding the token can use, which is the entire
/// difference between a server-side session and a signed one.
///
/// Deliberately does not require [`CurrentUser`]. Signing out has to work when
/// the session has already expired or been revoked elsewhere, and a 401 in
/// response to "sign me out" leaves the cookie in place. Nor is it a CSRF
/// concern: revoking a session requires possessing its token, and a
/// `SameSite=Lax` cookie is not sent on a cross-site POST.
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

/// The current user and their linked providers (T11).
///
/// D2 makes the list load-bearing rather than decorative: a user whose only
/// provider becomes inaccessible has no recovery path this service controls, so
/// seeing how many they have is what prompts them to link a second.
async fn me(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<MeResponse>> {
    // Sessions cascade from `users`, so a valid session for a deleted account
    // cannot exist. If it somehow does, the honest answer is "you are not
    // signed in" rather than a 500 about a missing row.
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

    Ok(Json(MeResponse {
        user: MeUser {
            id: account.id,
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

/// The session cookie.
///
/// `HttpOnly` because no script has any use for it and an XSS that can read it
/// owns the account. `Secure` from config, so local plain-HTTP development can
/// turn it off explicitly and every other deployment cannot forget to turn it
/// on. `SameSite=Lax` rather than `Strict`: `Lax` still withholds the cookie
/// from cross-site POSTs — which is what protects `/auth/link/{provider}` and
/// `/auth/logout` — while still sending it on a top-level navigation, so a
/// shared voyage link opens signed in. `Path=/` because the whole API is behind
/// it.
///
/// No `Max-Age`: `cookie`'s `time::Duration` is not nameable from this crate
/// (the `cookie` crate reaches us only through axum-extra's re-exports, which
/// stop at `Cookie`), and stringly-typed cookie building is not worth the
/// bug it invites. The consequence is a cookie that ends with the browser
/// session even though the server-side row is good for longer, which is a
/// safe-direction failure but not the intended one — see the phase notes.
pub(crate) fn session_cookie(config: &SessionConfig, token: &SessionToken) -> Cookie<'static> {
    Cookie::build((config.cookie_name.clone(), token.expose().to_owned()))
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(SameSite::Lax)
        .path("/")
        // Without a Max-Age this is a *browser-session* cookie: it is discarded
        // when the browser closes, while the server row lives for weeks. The
        // sliding expiry would then be silently defeated by closing a window.
        // Matched to the idle timeout, because that is when the row stops
        // resolving anyway — a cookie outliving its row only produces a
        // confusing 401.
        .max_age(cookie::time::Duration::seconds(clamp_secs(
            config.idle_timeout_secs,
        )))
        .build()
}

/// Name of the browser-binding cookie. Distinct from the session cookie, and
/// deliberately not configurable — nothing outside this module should be
/// reasoning about it.
const BINDING_COOKIE: &str = "pv_auth_binding";

/// The cookie that ties an authorization attempt to the browser that began it.
///
/// Three attributes are load-bearing and none of them is the obvious choice:
///
/// * **`SameSite=None`.** Apple's callback is a cross-site form POST, and a
///   `Lax` cookie is not sent on one — every Apple sign-in would fail the
///   binding check. `None` is what makes the mechanism work across all four
///   providers rather than three.
/// * **`Secure`.** Mandatory alongside `SameSite=None`; browsers reject the
///   combination otherwise. Set unconditionally rather than from
///   `cookie_secure`, because a `SameSite=None` cookie without `Secure` is not
///   a weaker cookie, it is a *dropped* one. Browsers make an explicit
///   exception for `http://localhost`, so local development still works.
/// * **`Max-Age` matched to the attempt TTL.** The cookie is worthless once the
///   attempt it names has expired, and leaving it behind means a browser
///   accumulates one per abandoned sign-in.
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

/// `u64` seconds to the `i64` the cookie crate wants, without wrapping a
/// configuration mistake into a negative duration — which would express
/// "already expired" and log nobody in.
fn clamp_secs(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Validates a caller-supplied destination, returning it only if it is a path
/// on this site that we recognise.
///
/// This is the open-redirect check. A sign-in URL is one users are expected to
/// click and share, so a `redirect_to` that can name another origin makes this
/// service a convincing first hop for a phishing flow — and the browser will
/// have shown our domain right up until the moment it leaves.
///
/// The check rejects rather than repairs. Normalising a hostile value is where
/// the interesting bypasses live: every layer that rewrites the string is
/// another chance for two parsers to disagree about what it means.
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

/// The destination for a completed flow.
///
/// Re-validated on the way out as well as on the way in. The check is cheap,
/// and a row written by an older build — or edited by hand — must not be able
/// to point a browser off-site.
fn destination(stored: Option<&str>, fallback: &str) -> String {
    stored
        .and_then(safe_redirect)
        .unwrap_or_else(|| fallback.to_owned())
}

/// An unknown provider name and one this deployment has no credentials for are
/// the same answer: 404. Distinguishing them would report which providers exist
/// but are switched off, which no caller needs to know.
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

/// Whether an error is the `(provider, subject)` unique index refusing a second
/// claim on one identity. That index is the real guarantee behind D3; the
/// application-level checks exist to produce a good message when it fires.
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
mod tests {
    use super::*;

    /// The values a phishing attempt actually uses. Each of these is a way of
    /// naming another origin with a string that starts with a slash, or of
    /// smuggling a second header past `Location`.
    #[test]
    fn off_site_destinations_are_refused() {
        for candidate in [
            "https://evil.example/",
            "//evil.example/",
            "/\\evil.example",
            "/\\/evil.example",
            "http:/evil.example",
            "evil.example",
            "/discover\r\nSet-Cookie: a=b",
            "/discover\nX: y",
            // Not on the allowlist, and on-site is not by itself enough: an
            // unknown path is a typo or a probe.
            "/admin",
            // Prefix without a separator — the reason the check looks at the
            // byte after the match.
            "/settings-evil.example",
            "/voyagesomething",
        ] {
            assert_eq!(safe_redirect(candidate), None, "accepted {candidate:?}");
        }
    }

    #[test]
    fn on_site_destinations_are_kept_verbatim() {
        for candidate in [
            "/",
            "/discover",
            "/settings",
            "/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
            "/discover?near=me#top",
        ] {
            assert_eq!(
                safe_redirect(candidate).as_deref(),
                Some(candidate),
                "refused {candidate:?}"
            );
        }
    }

    #[test]
    fn an_over_long_destination_is_refused() {
        let long = format!("/discover?q={}", "a".repeat(MAX_REDIRECT_LEN));
        assert_eq!(safe_redirect(&long), None);
    }

    /// A stored value is validated again on use, so an unvalidated row cannot
    /// become a redirect.
    #[test]
    fn a_stored_destination_falls_back_when_it_no_longer_passes() {
        assert_eq!(
            destination(Some("https://evil.example/"), SIGNED_IN_DESTINATION),
            SIGNED_IN_DESTINATION
        );
        assert_eq!(destination(None, LINKED_DESTINATION), LINKED_DESTINATION);
        assert_eq!(
            destination(Some("/settings"), SIGNED_IN_DESTINATION),
            "/settings"
        );
    }

    /// Apple's body is form-encoded, and its `user` payload is JSON inside one
    /// of the fields — so the field parser has to survive `{`, `"` and `%`.
    #[test]
    fn form_fields_are_read_from_an_apple_style_body() {
        let body = "state=abc&code=xyz&user=%7B%22name%22%3A%7B%22firstName%22%3A%22Ada%22%7D%7D";
        let fields: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();

        assert_eq!(field(&fields, "state"), Some("abc"));
        assert_eq!(field(&fields, "code"), Some("xyz"));
        assert_eq!(field(&fields, "error"), None);
        assert_eq!(
            field(&fields, "user"),
            Some(r#"{"name":{"firstName":"Ada"}}"#)
        );
    }
}
