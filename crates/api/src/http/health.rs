//! Liveness and readiness.
//!
//! These are deliberately different things. `/health` answers "is this process
//! alive" and must not touch the database — otherwise a database blip causes
//! orchestrators to kill healthy processes instead of routing around them.
//! `/ready` answers "can this process serve traffic" and does check it.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::db;
use crate::state::AppState;

/// Liveness. Always 200 while the process is running.
///
/// There is nothing to check here, and that is the point — the only honest
/// answer to "is this process alive" from code running inside that process is
/// "yes". Any dependency check added here turns a dependency outage into a
/// restart loop: the orchestrator kills every replica for being unhealthy,
/// they all come back, and none of them can reach the database either. The
/// database check lives in [`ready`], where a failure means "stop sending me
/// traffic" instead of "kill me".
pub async fn health() -> Response {
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

/// Readiness. 200 when the database round-trips, 503 when it does not.
///
/// A 503 here takes this instance out of the load balancer's rotation without
/// killing it, so it rejoins by itself when the database comes back.
///
/// Both probes build their response by hand rather than returning
/// `AppResult<T>` and letting [`crate::AppError`] map it. Two reasons: the
/// status code *is* the contract for a probe, and routing through `AppError`
/// would tie it to a mapping table where, for instance,
/// `AppError::Database(sqlx::Error::Io)` — a database that is simply down —
/// renders as a 500. That would be a silently broken readiness probe.
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
