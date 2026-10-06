use std::{error::Error, io};

use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
use serde_json::Value;

use super::{AppError, SourceChain};

async fn response(error: AppError) -> (StatusCode, Value) {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .finish();
    let response = tracing::subscriber::with_default(subscriber, || error.into_response());
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn caller_errors_keep_their_status_code_and_safe_message() {
    for (error, status, code, message) in [
        (
            AppError::NotFound,
            StatusCode::NOT_FOUND,
            "not_found",
            "The requested resource was not found.",
        ),
        (
            AppError::BadRequest("A field is invalid.".into()),
            StatusCode::BAD_REQUEST,
            "bad_request",
            "A field is invalid.",
        ),
        (
            AppError::Unauthorized,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication is required.",
        ),
        (
            AppError::Forbidden,
            StatusCode::FORBIDDEN,
            "forbidden",
            "You do not have access to this resource.",
        ),
        (
            AppError::Conflict("The voyage changed.".into()),
            StatusCode::CONFLICT,
            "conflict",
            "The voyage changed.",
        ),
    ] {
        let (actual_status, body) = response(error).await;
        assert_eq!(actual_status, status);
        assert_eq!(body["code"], code);
        assert_eq!(body["message"], message);
    }
}

#[tokio::test]
async fn retryable_errors_are_service_unavailable_and_hide_details() {
    for error in [
        AppError::ServiceUnavailable,
        AppError::Database(sqlx::Error::PoolTimedOut),
        AppError::Database(sqlx::Error::PoolClosed),
    ] {
        let (status, body) = response(error).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "service_unavailable");
        assert_eq!(
            body["message"],
            "The service is temporarily unavailable. Please try again."
        );
    }
}

#[tokio::test]
async fn internal_and_non_retryable_database_errors_hide_details() {
    for error in [
        AppError::Internal(Box::new(io::Error::other("private internal detail"))),
        AppError::Database(sqlx::Error::Protocol("private database detail".into())),
    ] {
        let (status, body) = response(error).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "internal");
        assert_eq!(
            body["message"],
            "An unexpected error occurred. Quote the request id if you report this."
        );
        assert!(!body.to_string().contains("private"));
    }
}

#[test]
fn source_chain_renders_each_cause_in_order() {
    #[derive(Debug)]
    struct Outer(io::Error);

    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "outer context")
        }
    }

    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    let error = Outer(io::Error::other("root cause"));
    assert_eq!(SourceChain(&error).to_string(), "outer context: root cause");
}
