//! Prepared, purpose-proven account deletion with independent outcome recovery.
use crate::{
    AppError, AppResult, AppState,
    auth::{CurrentUser, session::SessionToken},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me/deletion-requests", post(prepare))
        .route("/api/me/deletion-requests/{id}/confirm", post(confirm))
        .route("/api/account-deletions/{id}", get(status))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preparation {
    proof_token: String,
    apple_proof_token: Option<String>,
    status_capability: String,
}
#[derive(sqlx::FromRow)]
struct Receipt {
    id: Uuid,
    account_id: Option<Uuid>,
    request_hash: Vec<u8>,
    state: String,
    expires_at: DateTime<Utc>,
    apple_subject_hash: Option<Vec<u8>>,
}
#[derive(Serialize)]
struct Prepared {
    id: Uuid,
    status: String,
    expires_at: DateTime<Utc>,
}
fn private(status: StatusCode, body: impl Serialize) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}
fn invalid() -> AppError {
    AppError::BadRequest("Authenticate again to confirm account deletion.".into())
}
async fn lock_account(connection: &mut sqlx::PgConnection, account: Uuid) -> AppResult<()> {
    let active: Option<bool> =
        sqlx::query_scalar("SELECT NOT suspended FROM users WHERE id=$1 FOR UPDATE")
            .bind(account)
            .fetch_optional(connection)
            .await?;
    if active == Some(true) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}
#[derive(sqlx::FromRow)]
struct Proof {
    provider: String,
    subject_hash: Vec<u8>,
}
async fn proof(
    connection: &mut sqlx::PgConnection,
    account: Uuid,
    token: &str,
) -> AppResult<Proof> {
    let proof:Proof=sqlx::query_as("UPDATE deletion_proofs SET used=true WHERE token_hash=$1 AND account_id=$2 AND NOT used AND expires_at>now() RETURNING provider,subject_hash").bind(SessionToken::from_cookie_value(token).hash()).bind(account).fetch_optional(&mut *connection).await?.ok_or_else(invalid)?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT subject FROM identities WHERE user_id=$1 AND provider=$2")
            .bind(account)
            .bind(&proof.provider)
            .fetch_optional(connection)
            .await?;
    if current
        .map(|subject| Sha256::digest(subject.as_bytes()).to_vec())
        .as_deref()
        != Some(&proof.subject_hash)
    {
        return Err(invalid());
    }
    Ok(proof)
}
async fn prepare(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<Preparation>,
) -> AppResult<Response> {
    let id = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::BadRequest("Use a UUID Idempotency-Key.".into()))?;
    if !(43..=128).contains(&body.status_capability.len())
        || !body
            .status_capability
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
    {
        return Err(AppError::BadRequest(
            "Create and save a random deletion-status capability before preparation.".into(),
        ));
    }
    let hash = Sha256::digest(
        serde_json::to_vec(&json!([
            user.user_id,
            body.proof_token,
            body.apple_proof_token,
            body.status_capability
        ]))
        .map_err(AppError::internal)?,
    )
    .to_vec();
    let mut tx = state.pool.begin().await?;
    lock_account(&mut tx, user.user_id).await?;
    let previous:Option<Receipt>=sqlx::query_as("SELECT id,account_id,request_hash,state,expires_at,apple_subject_hash FROM account_deletions WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    if let Some(receipt) = previous {
        if receipt.account_id != Some(user.user_id) || receipt.request_hash != hash {
            return Err(AppError::Conflict(
                "This preparation key was already used.".into(),
            ));
        }
        return Ok(private(
            StatusCode::CREATED,
            Prepared {
                id,
                status: receipt.state,
                expires_at: receipt.expires_at,
            },
        ));
    }
    let first = proof(&mut tx, user.user_id, &body.proof_token).await?;
    let apple = if first.provider == "apple" {
        Some(first.subject_hash)
    } else if let Some(token) = body.apple_proof_token {
        let apple = proof(&mut tx, user.user_id, &token).await?;
        if apple.provider != "apple" {
            return Err(invalid());
        }
        Some(apple.subject_hash)
    } else {
        None
    };
    let apple_subject: Option<String> =
        sqlx::query_scalar("SELECT subject FROM identities WHERE user_id=$1 AND provider='apple'")
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if apple_subject
        .as_ref()
        .is_some_and(|subject| Some(Sha256::digest(subject.as_bytes()).to_vec()) != apple)
    {
        return Err(AppError::Conflict(
            "Verify your linked Apple account to revoke its authorization before deletion.".into(),
        ));
    }
    let expires:DateTime<Utc>=sqlx::query_scalar("INSERT INTO account_deletions(id,account_id,request_hash,status_hash,apple_subject_hash,state,expires_at) VALUES($1,$2,$3,$4,$5,'prepared',now()+interval '10 minutes') RETURNING expires_at").bind(id).bind(user.user_id).bind(hash).bind(SessionToken::from_cookie_value(&body.status_capability).hash()).bind(apple).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(private(
        StatusCode::CREATED,
        Prepared {
            id,
            status: "prepared".into(),
            expires_at: expires,
        },
    ))
}
async fn confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    let mut tx = state.pool.begin().await?;
    // Match administrator actions: serialize role eligibility before account locks.
    sqlx::query("SELECT pg_advisory_xact_lock(1919510126, 1)")
        .execute(&mut *tx)
        .await?;
    lock_account(&mut tx, user.user_id).await?;
    let receipt:Receipt=sqlx::query_as("SELECT id,account_id,request_hash,state,expires_at,apple_subject_hash FROM account_deletions WHERE id=$1 AND account_id=$2 AND retain_until>now() FOR UPDATE").bind(id).bind(user.user_id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if receipt.state != "prepared" {
        return Ok(private(
            StatusCode::ACCEPTED,
            json!({"id":receipt.id,"status":receipt.state}),
        ));
    }
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *tx)
        .await?;
    if receipt.expires_at <= now {
        return Err(invalid());
    }
    let apple: Option<String> =
        sqlx::query_scalar("SELECT subject FROM identities WHERE user_id=$1 AND provider='apple'")
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if apple.map(|subject| Sha256::digest(subject.as_bytes()).to_vec())
        != receipt.apple_subject_hash
    {
        return Err(invalid());
    }
    let last_admin:bool=sqlx::query_scalar("SELECT is_site_admin AND NOT EXISTS(SELECT 1 FROM users other WHERE other.id<>$1 AND other.is_site_admin AND NOT other.suspended) FROM users WHERE id=$1").bind(user.user_id).fetch_one(&mut *tx).await?;
    if last_admin {
        return Err(AppError::Conflict(
            "Assign another active site administrator before deleting this account.".into(),
        ));
    }
    let hashes = crate::accounts::deletion::remove(&mut tx, user.user_id).await?;
    let status = if hashes.is_empty() {
        "complete"
    } else {
        "deleting"
    };
    sqlx::query("UPDATE account_deletions SET state=$2,hashes=$3,account_id=NULL WHERE id=$1")
        .bind(id)
        .bind(status)
        .bind(hashes)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(private(
        StatusCode::ACCEPTED,
        json!({"id":id,"status":status}),
    ))
}
async fn status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let values = headers.get_all(header::AUTHORIZATION);
    let mut values = values.iter();
    let token = values
        .next()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Deletion "))
        .filter(|value| !value.is_empty() && !value.bytes().any(|b| b.is_ascii_whitespace()))
        .ok_or(AppError::NotFound)?;
    if values.next().is_some() {
        return Err(AppError::NotFound);
    }
    let result:Option<(String,DateTime<Utc>)>=sqlx::query_as("SELECT CASE WHEN state='prepared' AND expires_at<=now() THEN 'expired' ELSE state END,retain_until FROM account_deletions WHERE id=$1 AND status_hash=$2 AND retain_until>now()").bind(id).bind(SessionToken::from_cookie_value(token).hash()).fetch_optional(&state.pool).await?;
    let (status, expires) = result.ok_or(AppError::NotFound)?;
    Ok(private(
        StatusCode::OK,
        json!({"status":status,"retain_until":expires}),
    ))
}
