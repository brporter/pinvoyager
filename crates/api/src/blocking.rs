//! CPU-heavy image work and blocking filesystem operations run on Tokio's
//! blocking pool. The conversion queue bounds CPU concurrency independently.
//!
//! Database queries stay async. Decode and encode hold no database connection.
//! Final file writes and removals own their transaction while blocking so
//! cancellation cannot release a content lock before the filesystem stops.

use crate::error::{AppError, AppResult};

/// Runs fallible CPU-bound or blocking work on the blocking thread pool.
///
/// The closure returns [`AppResult<T>`] rather than an arbitrary `Result`
/// deliberately: whether a failed image decode is a `400` (the user uploaded
/// something that is not an image) or a `500` (the encoder fell over on valid
/// input) is knowledge the *call site* has and this module does not. Mapping
/// the domain error at the point where the context exists keeps the result
/// flat — `run_blocking(...).await?` yields a `T` — instead of the
/// `AppResult<Result<T, E>>` that a `FnOnce() -> T` signature forces callers
/// to unwrap twice.
///
/// It also avoids needing a `From<ImageError> for AppError` impl (and one per
/// future library) purely to satisfy a generic bound.
///
/// Panics inside `f` are surfaced as [`AppError::Internal`] rather than taking
/// down the worker.
///
/// ```no_run
/// # use pinvoyager_api::{blocking::run_blocking, error::{AppError, AppResult}};
/// # async fn example(bytes: Vec<u8>) -> AppResult<Vec<u8>> {
/// let thumbnail = run_blocking(move || {
///     // decode -> resize -> re-encode, all off the runtime
///     decode_and_resize(&bytes).map_err(|e| AppError::BadRequest(e.to_string()))
/// })
/// .await?;
/// # Ok(thumbnail)
/// # }
/// # fn decode_and_resize(_: &[u8]) -> Result<Vec<u8>, std::io::Error> { unimplemented!() }
/// ```
pub async fn run_blocking<F, T>(f: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    // Map a failed blocking task join to an internal application error.
    let span = tracing::Span::current();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    match tokio::task::spawn_blocking(move || {
        tracing::dispatcher::with_default(&dispatch, || span.in_scope(f))
    })
    .await
    {
        Ok(result) => result,
        Err(join_err) => Err(AppError::from_join(join_err)),
    }
}

/// Runs an infallible closure on the blocking thread pool.
/// The returned result reports task join failures, including panics.
pub async fn run_blocking_infallible<F, T>(f: F) -> AppResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let span = tracing::Span::current();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    tokio::task::spawn_blocking(move || {
        tracing::dispatcher::with_default(&dispatch, || span.in_scope(f))
    })
    .await
    .map_err(AppError::from_join)
}

impl AppError {
    /// Helper for the `JoinError` produced when a blocking task panics.
    pub fn from_join(err: tokio::task::JoinError) -> Self {
        AppError::Internal(Box::new(err))
    }
}

#[cfg(test)]
#[path = "blocking_tests.rs"]
mod tests;
