//! Liveness checks the process without dependencies. Readiness checks PostgreSQL.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::db;
use crate::state::AppState;

/// Returns 200 without contacting the database.
pub async fn health() -> Response {
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

/// Returns 200 when the database query succeeds and 503 when it fails.
/// Database errors are logged without exposing connection details in the body.
pub async fn ready(State(state): State<AppState>) -> Response {
    match db::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
        Err(err) => {
            // The cause goes to the log, never to the body: a connection error
            // string carries the host, port and database name, and this
            // endpoint is reachable by anything that can reach the service.
            // The caller learns only that we are not ready.
            tracing::warn!(error = %err, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unavailable" })),
            )
                .into_response()
        }
    }
}
