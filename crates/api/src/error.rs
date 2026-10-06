//! The single application error type.
//!
//! CONTRACT — variants are fixed; handlers across phases 3–5 return
//! `AppResult<T>`. The `IntoResponse` implementation is owned by the config /
//! error agent.
//!
//! The rule this type exists to enforce: **the client learns the status code
//! and a generic message; the log learns everything.** Internal error text
//! never reaches a response body. A caller who needs to report a problem
//! quotes the request id.

use std::fmt;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    /// A dependency is unavailable — most commonly pool acquisition timing
    /// out under load (D8). Distinct from `Internal` because it is retryable
    /// and should not page anyone on its own.
    #[error("service unavailable")]
    ServiceUnavailable,

    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AppError {
    /// Status plus a stable machine-readable code.
    ///
    /// The codes are part of the API surface — clients are expected to branch
    /// on them rather than on prose — so they are written out here instead of
    /// being derived from variant names, which would make a rename a silent
    /// breaking change.
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::ServiceUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable")
            }
            // D8: pool exhaustion is not an internal fault, it is load. It is
            // retryable, the client should be told so, and it should not be
            // counted alongside genuine 500s when deciding whether to page.
            AppError::Database(sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed) => {
                (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable")
            }
            AppError::Database(_) | AppError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        }
    }

    /// What the client is allowed to read.
    ///
    /// For 5xx this is fixed prose: a sqlx message can name a table, a column,
    /// or a constraint, and none of that is the client's business. For the two
    /// 4xx variants that carry a string the string is included, because those
    /// are written by a handler for the caller — there is no `#[from]` on
    /// either variant, so nothing can turn an internal error into one by
    /// accident.
    fn client_message(&self) -> &str {
        match self {
            AppError::NotFound => "The requested resource was not found.",
            AppError::BadRequest(msg) => msg,
            AppError::Unauthorized => "Authentication is required.",
            AppError::Forbidden => "You do not have access to this resource.",
            AppError::Conflict(msg) => msg,
            _ => match self.status_and_code().0 {
                StatusCode::SERVICE_UNAVAILABLE => {
                    "The service is temporarily unavailable. Please try again."
                }
                _ => "An unexpected error occurred. Quote the request id if you report this.",
            },
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();

        // The log line is the other half of the contract: everything the
        // response withholds has to land here, correlated by the request id
        // that the middleware layer puts on the surrounding span and on the
        // `x-request-id` response header.
        let status_code = status.as_u16();
        if status.is_server_error() {
            let error_message = self.to_string();
            let error_chain = SourceChain(&self).to_string();
            tracing::error!(
                status = status_code,
                code,
                error = %error_message,
                error.chain = %error_chain,
                "request failed"
            );
        } else {
            // A 4xx is the client's mistake, not ours. Logging these at error
            // level trains people to ignore error level.
            let error_message = self.to_string();
            tracing::debug!(
                status = status_code,
                code,
                error = %error_message,
                "request rejected"
            );
        }

        (
            status,
            Json(json!({ "code": code, "message": self.client_message() })),
        )
            .into_response()
    }
}

/// Renders an error together with its `source` chain on one line.
///
/// `Display` on the outer error gives only the top frame — `AppError::Database`
/// prints "database error" and nothing else. The frame that says *why* is
/// underneath it, and it is the only reason the log line exists.
struct SourceChain<'a>(&'a (dyn std::error::Error + 'static));

impl fmt::Display for SourceChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(cause) = source {
            write!(f, ": {cause}")?;
            source = cause.source();
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
