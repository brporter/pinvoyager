//! Native Apple authorization reuses account policy and the app PKCE session exchange.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AttemptBody {
    mode: String,
    state: String,
    challenge: String,
}

pub(super) async fn create(
    State(state): State<AppState>,
    user: MaybeUser,
    headers: HeaderMap,
    Json(body): Json<AttemptBody>,
) -> AppResult<Response> {
    if !state
        .providers
        .get(ProviderKind::Apple)
        .is_some_and(|provider| provider.supports_native_apple())
    {
        return Err(AppError::NotFound);
    }
    if !matches!(body.mode.as_str(), "sign_in" | "link" | "reauth_delete")
        || !verifier_valid(&body.state)
        || URL_SAFE_NO_PAD
            .decode(&body.challenge)
            .ok()
            .is_none_or(|value| value.len() != 32)
    {
        return Err(invalid());
    }
    let owner = if body.mode == "sign_in" {
        None
    } else {
        native_token(&headers, &state.config.auth.session.cookie_name)?
            .ok_or(AppError::Unauthorized)?;
        Some(user.0.ok_or(AppError::Unauthorized)?.user_id)
    };
    let id = Uuid::now_v7();
    let nonce = SessionToken::generate();
    sqlx::query("INSERT INTO native_auth_attempts(id,provider,mode,app_state,challenge,callback_identifier,user_id,expires_at,started_at,apple_nonce) VALUES($1,'apple',$2,$3,$4,'pinvoyager',$5,now()+interval '10 minutes',now(),$6)")
        .bind(id).bind(body.mode).bind(&body.state).bind(body.challenge).bind(owner).bind(nonce.expose()).execute(&state.pool).await?;
    Ok(no_store(
        StatusCode::CREATED,
        serde_json::json!({"attempt_id":id,"state":body.state,"nonce":nonce.expose()}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompleteBody {
    attempt_id: Uuid,
    state: String,
    verifier: String,
    authorization_code: String,
}

pub(super) async fn complete(
    State(state): State<AppState>,
    Json(body): Json<CompleteBody>,
) -> AppResult<Response> {
    if !verifier_valid(&body.verifier)
        || body.authorization_code.is_empty()
        || body.authorization_code.len() > 4096
    {
        return Err(invalid());
    }
    let provider = state
        .providers
        .get(ProviderKind::Apple)
        .filter(|provider| provider.supports_native_apple())
        .ok_or(AppError::NotFound)?;
    // Consume before contacting Apple so concurrent requests cannot reuse an attempt.
    // A failed or lost exchange requires a fresh authorization, just like browser sign-in.
    let nonce: String = sqlx::query_scalar("WITH candidate AS (SELECT id,apple_nonce FROM native_auth_attempts WHERE id=$1 AND provider='apple' AND apple_nonce IS NOT NULL AND app_state=$2 AND challenge=$3 AND expires_at>now() AND redeemed_at IS NULL FOR UPDATE) UPDATE native_auth_attempts a SET apple_nonce=NULL FROM candidate c WHERE a.id=c.id RETURNING c.apple_nonce")
        .bind(body.attempt_id).bind(&body.state).bind(URL_SAFE_NO_PAD.encode(Sha256::digest(body.verifier.as_bytes())))
        .fetch_optional(&state.pool).await?.flatten().ok_or_else(invalid)?;
    let subject = reauth_subject(&state, body.attempt_id, ProviderKind::Apple).await?;
    let identity = provider
        .exchange_native_apple(&body.authorization_code, &nonce, subject.as_deref())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "native Apple verification failed");
            invalid()
        })?;
    if identity.provider != ProviderKind::Apple {
        return Err(invalid());
    }
    let callback = complete_verified(&state, body.attempt_id, &identity).await?;
    let code = callback
        .query_pairs()
        .find(|(name, _)| name == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(invalid)?;
    exchange(
        State(state),
        Json(ExchangeBody {
            attempt_id: body.attempt_id,
            code,
            verifier: body.verifier,
        }),
    )
    .await
}
