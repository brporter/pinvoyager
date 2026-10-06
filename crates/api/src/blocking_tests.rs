use super::{run_blocking, run_blocking_infallible};
use crate::error::{AppError, AppResult};

#[tokio::test]
async fn returns_the_closure_value() {
    let out = run_blocking(|| Ok(2 + 2)).await;
    assert!(matches!(out, Ok(4)));
}

#[tokio::test]
async fn propagates_the_closure_error() {
    let out: AppResult<()> = run_blocking(|| Err(AppError::BadRequest("nope".into()))).await;
    assert!(matches!(out, Err(AppError::BadRequest(_))));
}

/// The point of the wrapper: a panic in a decoder becomes a 500, and the
/// runtime keeps its worker.
#[tokio::test]
async fn panic_becomes_internal() {
    let out: AppResult<()> = run_blocking(|| panic!("decoder exploded")).await;
    assert!(matches!(out, Err(AppError::Internal(_))));

    // ...and the runtime is still usable afterwards.
    let after = run_blocking_infallible(|| "still here").await;
    assert!(matches!(after, Ok("still here")));
}

#[tokio::test]
async fn infallible_form_needs_no_error_annotation() {
    let out = run_blocking_infallible(|| vec![1u8, 2, 3].len()).await;
    assert!(matches!(out, Ok(3)));
}
