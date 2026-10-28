//! App PKCE handoff around the shared, browser-bound provider verification.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::{
    AppError, AppResult, AppState,
    auth::{
        MaybeUser, ProviderKind, VerifiedIdentity,
        extract::native_token,
        session::{self, SessionToken},
        sign_in::{self, SignInIdentity},
    },
    db::repo::users,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/client-capabilities", get(capabilities))
        .route("/auth/native/attempts", post(create_attempt))
        .route("/auth/native/reauth-attempts", post(create_reauth))
        .route("/auth/native/attempts/{id}/start", get(start))
        .route("/auth/native/exchange", post(exchange))
        .route("/auth/native/logout", post(logout))
}

fn no_store(status: StatusCode, body: impl Serialize) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

async fn capabilities(State(state): State<AppState>) -> Response {
    no_store(
        StatusCode::OK,
        serde_json::json!({
            "native_authentication": true,
            "native_protocol_versions": [1],
            "providers":state.providers.configured().iter().map(|kind|kind.as_str()).collect::<Vec<_>>(),
            "formats":["heic","jpeg","png","webp"],
            "upload_timeout_secs":state.config.server.upload_request_timeout_secs,
            "max_upload_bytes":state.config.photos.max_upload_bytes,
            "max_pixels":u64::from(state.config.photos.max_megapixels)*1_000_000,
            "max_order_entries":10_000,
            "max_manifest_entries":10_000,
            "owner_offline_downloads":true,
            "canonical_web_origin":state.config.auth.base_url.origin().ascii_serialization()
        }),
    )
}

#[derive(Deserialize)]
struct AttemptBody {
    provider: String,
    mode: String,
    state: String,
    challenge: String,
    callback_identifier: String,
}

fn invalid() -> AppError {
    AppError::BadRequest(sign_in::GENERIC_FAILURE.to_owned())
}

fn verifier_valid(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

async fn create_attempt(
    State(state): State<AppState>,
    user: MaybeUser,
    headers: HeaderMap,
    Json(body): Json<AttemptBody>,
) -> AppResult<Response> {
    create(&state, user, headers, body, false).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReauthBody {
    provider: String,
    purpose: String,
    state: String,
    challenge: String,
    callback_identifier: String,
}
async fn create_reauth(
    State(state): State<AppState>,
    user: MaybeUser,
    headers: HeaderMap,
    Json(body): Json<ReauthBody>,
) -> AppResult<Response> {
    if body.purpose != "account_deletion" {
        return Err(invalid());
    }
    create(
        &state,
        user,
        headers,
        AttemptBody {
            provider: body.provider,
            mode: "reauth_delete".into(),
            state: body.state,
            challenge: body.challenge,
            callback_identifier: body.callback_identifier,
        },
        true,
    )
    .await
}
async fn create(
    state: &AppState,
    user: MaybeUser,
    headers: HeaderMap,
    body: AttemptBody,
    reauth: bool,
) -> AppResult<Response> {
    let provider = super::parse_provider(&body.provider)?;
    if state.providers.get(provider).is_none() {
        return Err(AppError::NotFound);
    }
    if body.callback_identifier != "pinvoyager"
        || !verifier_valid(&body.state)
        || (!matches!(body.mode.as_str(), "sign_in" | "link")
            && !(reauth && body.mode == "reauth_delete"))
        || URL_SAFE_NO_PAD
            .decode(&body.challenge)
            .ok()
            .is_none_or(|bytes| bytes.len() != 32)
    {
        return Err(invalid());
    }
    let user_id = if body.mode == "link" || reauth {
        native_token(&headers, &state.config.auth.session.cookie_name)?
            .ok_or(AppError::Unauthorized)?;
        Some(user.0.ok_or(AppError::Unauthorized)?.user_id)
    } else {
        None
    };
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO native_auth_attempts(id,provider,mode,app_state,challenge,callback_identifier,user_id,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,now()+interval '10 minutes')")
        .bind(id).bind(provider.as_str()).bind(body.mode).bind(body.state).bind(body.challenge).bind(body.callback_identifier).bind(user_id)
        .execute(&state.pool).await?;
    let url = state
        .config
        .auth
        .base_url
        .join(&format!("/auth/native/attempts/{id}/start"))
        .map_err(|_| invalid())?;
    Ok(no_store(
        StatusCode::CREATED,
        serde_json::json!({"attempt_id":id,"browser_start_url":url}),
    ))
}

#[derive(sqlx::FromRow)]
struct NativeAttempt {
    id: Uuid,
    provider: String,
    mode: String,
    app_state: String,
    challenge: String,
    user_id: Option<Uuid>,
}

async fn start(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    jar: CookieJar,
) -> AppResult<Response> {
    let attempt: NativeAttempt=sqlx::query_as("UPDATE native_auth_attempts SET started_at=now() WHERE id=$1 AND started_at IS NULL AND expires_at>now() RETURNING id,provider,mode,app_state,challenge,user_id")
        .bind(id).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
    let provider = ProviderKind::parse(&attempt.provider).ok_or_else(invalid)?;
    let (url, binding) = super::begin(&state, provider, attempt.user_id, None, Some(id)).await?;
    let provider_state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(invalid)?;
    sqlx::query("UPDATE native_auth_attempts SET provider_state=$2 WHERE id=$1")
        .bind(id)
        .bind(provider_state)
        .execute(&state.pool)
        .await?;
    Ok((
        jar.add(super::binding_cookie(&binding)),
        super::redirect(StatusCode::FOUND, url.as_str())?,
    )
        .into_response())
}

/// Called only after provider, nonce, provider PKCE, state and browser binding verification.
pub(super) async fn complete_verified(
    state: &AppState,
    id: Uuid,
    identity: &VerifiedIdentity,
) -> AppResult<url::Url> {
    let attempt: NativeAttempt=sqlx::query_as("SELECT id,provider,mode,app_state,challenge,user_id FROM native_auth_attempts WHERE id=$1 AND expires_at>now() AND handoff_hash IS NULL AND started_at IS NOT NULL")
        .bind(id).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
    if attempt.provider != identity.provider.as_str() {
        return Err(invalid());
    }
    let mut tx = state.pool.begin().await?;
    let user_id = if attempt.mode == "reauth_delete" {
        let owner = attempt.user_id.ok_or_else(invalid)?;
        users::require_active(&mut tx, owner).await?;
        let matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM identities WHERE user_id=$1 AND provider=$2 AND subject=$3)").bind(owner).bind(identity.provider.as_str()).bind(&identity.subject).fetch_one(&mut *tx).await?;
        if !matches {
            return Err(invalid());
        }
        owner
    } else if attempt.mode == "link" {
        let user_id = attempt.user_id.ok_or_else(invalid)?;
        super::link_in_transaction(&mut tx, user_id, identity).await?;
        user_id
    } else {
        sign_in::resolve_identity(&mut tx, SignInIdentity::Verified(identity)).await?
    };
    let code = SessionToken::generate();
    let result=sqlx::query("UPDATE native_auth_attempts SET user_id=$2,handoff_hash=$3,code_expires_at=now()+interval '60 seconds' WHERE id=$1 AND expires_at>now() AND handoff_hash IS NULL AND redeemed_at IS NULL")
        .bind(attempt.id).bind(user_id).bind(code.hash()).execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Err(invalid());
    }
    tx.commit().await?;
    let mut target = url::Url::parse("pinvoyager://auth/callback").map_err(|_| invalid())?;
    target
        .query_pairs_mut()
        .append_pair("code", code.expose())
        .append_pair("state", &attempt.app_state);
    Ok(target)
}

#[derive(Deserialize)]
struct ExchangeBody {
    attempt_id: Uuid,
    code: String,
    verifier: String,
}

#[derive(Serialize)]
struct ExchangeResponse {
    session_token: String,
    #[serde(flatten)]
    account: super::MeResponse,
    idle_deadline: DateTime<Utc>,
    absolute_deadline: DateTime<Utc>,
}

async fn exchange(
    State(state): State<AppState>,
    Json(body): Json<ExchangeBody>,
) -> AppResult<Response> {
    if !verifier_valid(&body.verifier) || body.code.len() > 128 {
        return Err(invalid());
    }
    let candidate: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM native_auth_attempts WHERE id=$1 AND user_id IS NOT NULL",
    )
    .bind(body.attempt_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let user_id = candidate.ok_or_else(invalid)?;
    let mut tx = state.pool.begin().await?;
    users::require_active(&mut tx, user_id).await?;
    let attempt: NativeAttempt=sqlx::query_as("SELECT id,provider,mode,app_state,challenge,user_id FROM native_auth_attempts WHERE id=$1 AND user_id=$2 AND expires_at>now() AND code_expires_at>now() AND redeemed_at IS NULL AND handoff_hash=$3 FOR UPDATE")
        .bind(body.attempt_id).bind(user_id).bind(SessionToken::from_cookie_value(&body.code).hash()).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(body.verifier.as_bytes()));
    if !constant_time_equal(challenge.as_bytes(), attempt.challenge.as_bytes()) {
        return Err(invalid());
    }
    if attempt.mode == "reauth_delete" {
        let subject: String =
            sqlx::query_scalar("SELECT subject FROM identities WHERE user_id=$1 AND provider=$2")
                .bind(user_id)
                .bind(&attempt.provider)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(invalid)?;
        let proof = SessionToken::generate();
        sqlx::query("INSERT INTO deletion_proofs(token_hash,account_id,provider,subject_hash,expires_at) VALUES($1,$2,$3,$4,now()+interval '10 minutes')")
            .bind(proof.hash()).bind(user_id).bind(&attempt.provider).bind(Sha256::digest(subject.as_bytes()).to_vec()).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE native_auth_attempts SET redeemed_at=now(),handoff_hash=NULL WHERE id=$1",
        )
        .bind(body.attempt_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(no_store(
            StatusCode::OK,
            serde_json::json!({"proof_token":proof.expose(),"account_id":user_id,"provider":attempt.provider,"purpose":"account_deletion","expires_at":Utc::now()+Duration::minutes(10)}),
        ));
    }
    let token = session::create(&mut *tx, user_id, &state.config.auth.session).await?;
    sqlx::query("INSERT INTO native_sessions(token_hash) VALUES($1)")
        .bind(token.hash())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE native_auth_attempts SET redeemed_at=now(),handoff_hash=NULL WHERE id=$1")
        .bind(body.attempt_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let session = session::lookup_and_touch(&state.pool, &token, &state.config.auth.session)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let idle = Duration::try_seconds(
        i64::try_from(state.config.auth.session.idle_timeout_secs).unwrap_or(i64::MAX),
    );
    let deadline = idle
        .and_then(|idle| session.last_seen_at.checked_add_signed(idle))
        .unwrap_or(session.absolute_expires_at)
        .min(session.absolute_expires_at);
    Ok(no_store(
        StatusCode::OK,
        ExchangeResponse {
            session_token: token.expose().to_owned(),
            account: super::account_response(&state, user_id).await?,
            idle_deadline: deadline,
            absolute_deadline: session.absolute_expires_at,
        },
    ))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    let token = native_token(&headers, &state.config.auth.session.cookie_name)?
        .ok_or(AppError::Unauthorized)?;
    sqlx::query("DELETE FROM sessions WHERE token_hash=$1 AND EXISTS(SELECT 1 FROM native_sessions n WHERE n.token_hash=sessions.token_hash)")
        .bind(token.hash()).execute(&state.pool).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::CACHE_CONTROL, "no-store")],
    )
        .into_response())
}

/// Reads the initiating account's linked subject before a deletion-only provider exchange.
pub(super) async fn reauth_subject(
    state: &AppState,
    id: Uuid,
    provider: ProviderKind,
) -> AppResult<Option<String>> {
    let attempt: NativeAttempt = sqlx::query_as("SELECT id,provider,mode,app_state,challenge,user_id FROM native_auth_attempts WHERE id=$1 AND expires_at>now() AND handoff_hash IS NULL AND started_at IS NOT NULL").bind(id).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
    if attempt.provider != provider.as_str() {
        return Err(invalid());
    }
    if attempt.mode != "reauth_delete" {
        return Ok(None);
    }
    let subject: String = sqlx::query_scalar("SELECT i.subject FROM identities i JOIN users u ON u.id=i.user_id WHERE i.user_id=$1 AND i.provider=$2 AND NOT u.suspended").bind(attempt.user_id).bind(provider.as_str()).fetch_optional(&state.pool).await?.ok_or_else(invalid)?;
    Ok(Some(subject))
}
