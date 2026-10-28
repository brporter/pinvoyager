//! Durable operation receipts and revision checks shared by native mutations.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{Acquire as _, PgConnection};
use uuid::Uuid;

use crate::{AppError, AppResult, AppState, auth::CurrentUser, db::repo::users};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/sync/operations/{id}", get(receipt))
}

pub fn conditional(headers: &HeaderMap) -> bool {
    headers.contains_key(header::AUTHORIZATION)
        || headers.contains_key(header::IF_MATCH)
        || headers.contains_key("Idempotency-Key")
}

pub struct Request {
    pub method: &'static str,
    pub path: String,
    pub payload: Value,
    pub requires_revision: bool,
}

pub struct Outcome {
    pub status: StatusCode,
    pub body: Vec<u8>,
    pub target_id: Uuid,
    pub voyage_id: Uuid,
    pub resource_kind: &'static str,
    pub revision: i64,
    pub previous_revision: Option<i64>,
    pub deleted: bool,
}

impl Outcome {
    pub fn json(
        status: StatusCode,
        target_id: Uuid,
        voyage_id: Uuid,
        resource_kind: &'static str,
        revision: i64,
        value: impl Serialize,
    ) -> AppResult<Self> {
        let body = serde_json::to_vec(&value).map_err(AppError::internal)?;
        Ok(Self {
            status,
            body,
            target_id,
            voyage_id,
            resource_kind,
            revision,
            deleted: false,
            previous_revision: None,
        })
    }
    pub fn empty(
        status: StatusCode,
        target_id: Uuid,
        voyage_id: Uuid,
        resource_kind: &'static str,
        revision: i64,
    ) -> Self {
        Self {
            status,
            body: Vec::new(),
            target_id,
            voyage_id,
            resource_kind,
            revision,
            deleted: false,
            previous_revision: None,
        }
    }
    pub fn deleted(
        target_id: Uuid,
        voyage_id: Uuid,
        resource_kind: &'static str,
        revision: i64,
    ) -> Self {
        Self {
            status: StatusCode::NO_CONTENT,
            body: Vec::new(),
            target_id,
            voyage_id,
            resource_kind,
            revision,
            deleted: true,
            previous_revision: None,
        }
    }
}

fn response(status: StatusCode, body: Vec<u8>) -> Response {
    let mut response = (status, Body::from(body)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    if status != StatusCode::NO_CONTENT {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
    }
    response
}

#[derive(sqlx::FromRow)]
struct Receipt {
    request_hash: Vec<u8>,
    state: String,
    target_id: Option<Uuid>,
    voyage_id: Option<Uuid>,
    result_revision: Option<i64>,
    previous_revision: Option<i64>,
    response_status: Option<i16>,
    response_body: Option<Vec<u8>>,
    resource_kind: Option<String>,
}

impl Receipt {
    fn summary(&self) -> Value {
        let mut value = json!({"status":self.state,"target_id":self.target_id,"voyage_id":self.voyage_id,"revision":self.result_revision.map(|revision|revision.to_string())});
        if let Some(previous) = self.previous_revision {
            value["previous_revision"] = json!(previous.to_string());
        }
        if self.resource_kind.as_deref() == Some("publication")
            && self.state == "applied"
            && let Some(bytes) = &self.response_body
            && let Ok(body) = serde_json::from_slice::<Value>(bytes)
        {
            value["publication_intent_id"] = json!(self.target_id);
            value["publication_status"] = body["status"].clone();
        }
        if self.state == "rejected"
            && let Some(bytes) = &self.response_body
            && let Ok(body) = serde_json::from_slice::<Value>(bytes)
            && let Some(message) = body.get("message")
        {
            value["message"] = message.clone();
            value["response_status"] = json!(self.response_status);
            value["code"] = body["code"].clone();
        }
        value
    }
    fn replay(self) -> AppResult<Response> {
        if self.state == "deleted" {
            return Ok(response(
                StatusCode::OK,
                serde_json::to_vec(&self.summary()).map_err(AppError::internal)?,
            ));
        }
        let status = self
            .response_status
            .and_then(|value| StatusCode::from_u16(value as u16).ok())
            .ok_or(AppError::ServiceUnavailable)?;
        Ok(response(status, self.response_body.unwrap_or_default()))
    }
}

/// Savepoints retain permanent rejection receipts while rolling back their attempted writes.
pub async fn run<E>(
    state: &AppState,
    user_id: Uuid,
    headers: &HeaderMap,
    request: Request,
    action: impl AsyncFnOnce(&mut PgConnection, Option<i64>) -> Result<Outcome, E>,
) -> Result<Response, E>
where
    E: From<AppError> + From<sqlx::Error> + IntoResponse,
{
    for name in ["Idempotency-Key", "If-Match"] {
        if headers.get_all(name).iter().count() > 1 {
            return Err(AppError::BadRequest(format!("Send one {name} header.")).into());
        }
    }
    let native = headers.contains_key(header::AUTHORIZATION);
    let key = headers
        .get("Idempotency-Key")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| AppError::BadRequest("Idempotency-Key must be a UUID.".to_owned()))
        })
        .transpose()?;
    if native && key.is_none() {
        return Err(AppError::BadRequest(
            "An Idempotency-Key is required for native mutations.".to_owned(),
        )
        .into());
    }
    let expected = headers
        .get(header::IF_MATCH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("\"v:")?.strip_suffix('"'))
                .filter(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                })
                .and_then(|value| value.parse::<i64>().ok())
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "If-Match must contain a strong voyage revision.".to_owned(),
                    )
                })
        })
        .transpose()?;
    if native && request.requires_revision && expected.is_none() {
        return Err(AppError::BadRequest(
            "If-Match is required for this native mutation.".to_owned(),
        )
        .into());
    }
    let checked_previous = if request.requires_revision {
        expected
    } else {
        None
    };
    let hash = Sha256::digest(
        serde_json::to_vec(&json!([
            request.method,
            request.path,
            expected.map(|revision| revision.to_string()),
            request.payload
        ]))
        .map_err(AppError::internal)?,
    )
    .to_vec();
    let mut tx = state.pool.begin().await?;
    users::require_active(&mut tx, user_id).await?;
    if let Some(key) = key {
        let inserted=sqlx::query("INSERT INTO native_operations(account_id,id,request_hash,state) VALUES($1,$2,$3,'in_progress') ON CONFLICT DO NOTHING").bind(user_id).bind(key).bind(&hash).execute(&mut *tx).await?.rows_affected();
        let receipt: Receipt=sqlx::query_as("SELECT request_hash,state,target_id,voyage_id,result_revision,previous_revision,response_status,response_body,resource_kind FROM native_operations WHERE account_id=$1 AND id=$2 FOR UPDATE").bind(user_id).bind(key).fetch_one(&mut *tx).await?;
        if receipt.request_hash != hash {
            return Err(AppError::Conflict(
                "This operation key was used for a different request.".to_owned(),
            )
            .into());
        }
        if inserted == 0 {
            return Ok(receipt.replay()?);
        }
    }
    let mut savepoint = tx.begin().await?;
    let result = action(&mut savepoint, expected).await;
    let (status, body, target, state_name) = match result {
        Ok(outcome) => {
            savepoint.commit().await?;
            (
                outcome.status,
                outcome.body,
                Some((
                    outcome.target_id,
                    outcome.voyage_id,
                    outcome.resource_kind,
                    outcome.revision,
                    outcome.previous_revision.or(checked_previous),
                )),
                if outcome.deleted {
                    "deleted"
                } else {
                    "applied"
                },
            )
        }
        Err(error) => {
            savepoint.rollback().await?;
            let error = error.into_response();
            let status = error.status();
            let retry_after = error.headers().get(header::RETRY_AFTER).cloned();
            let bytes = to_bytes(error.into_body(), 64 * 1024)
                .await
                .map_err(AppError::internal)?
                .to_vec();
            if status.is_server_error()
                || matches!(
                    status,
                    StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
                )
            {
                let mut response = response(status, bytes);
                if let Some(delay) = retry_after {
                    response.headers_mut().insert(header::RETRY_AFTER, delay);
                }
                return Ok(response);
            }
            (status, bytes, None, "rejected")
        }
    };
    if let Some(key) = key {
        let (target_id, voyage_id, kind, revision, previous_revision) = target
            .map(|(id, voyage, kind, revision, previous)| {
                (Some(id), Some(voyage), Some(kind), Some(revision), previous)
            })
            .unwrap_or_default();
        sqlx::query("UPDATE native_operations SET state=$3,target_id=$4,voyage_id=$5,resource_kind=$6,result_revision=$7,response_status=$8,response_body=$9,previous_revision=$10 WHERE account_id=$1 AND id=$2")
            .bind(user_id).bind(key).bind(state_name).bind(target_id).bind(voyage_id).bind(kind).bind(revision).bind(status.as_u16() as i16).bind(&body).bind(previous_revision).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(response(status, body))
}

pub async fn revision(
    connection: &mut PgConnection,
    voyage_id: Uuid,
    expected: Option<i64>,
) -> AppResult<i64> {
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM voyages WHERE id=$1")
        .bind(voyage_id)
        .fetch_optional(connection)
        .await?
        .ok_or(AppError::NotFound)?;
    if expected.is_some_and(|expected| expected != revision) {
        return Err(AppError::PreconditionFailed);
    }
    Ok(revision)
}

async fn receipt(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    let receipt: Option<Receipt>=sqlx::query_as("SELECT request_hash,state,target_id,voyage_id,result_revision,previous_revision,response_status,response_body,resource_kind FROM native_operations WHERE account_id=$1 AND id=$2").bind(user.user_id).bind(id).fetch_optional(&state.pool).await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(
            receipt
                .map(|receipt| receipt.summary())
                .unwrap_or_else(|| json!({"status":"not_seen"})),
        ),
    )
        .into_response())
}

#[cfg(test)]
#[path = "mutations_tests.rs"]
mod tests;
