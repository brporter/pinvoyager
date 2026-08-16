//! Blocking-work discipline (D8).
//!
//! An async runtime multiplexes many tasks onto few OS threads. A task that
//! occupies a thread without yielding stalls every other task scheduled on it.
//! In a service that mostly awaits Postgres this is invisible — until phase 4
//! starts decoding photographs, at which point a fifty-file batch will make
//! unrelated requests time out.
//!
//! Every CPU-bound or blocking operation goes through this module, so that
//! call sites are uniform and grep-able. If you find yourself decoding an
//! image, hashing a large buffer, or touching `std::fs` outside here, that is
//! the bug.
//!
//! # What belongs here
//!
//! Concretely, from D8's hazard list:
//!
//! * image decode, resize and encode — the single biggest risk, because the
//!   work is proportional to *pixels*, not to bytes on the wire: one 25 MB
//!   JPEG is tens of milliseconds of pure CPU, and fifty of them arriving as a
//!   batch is seconds of a worker thread that unrelated requests are queued
//!   behind;
//! * content hashing of upload-sized buffers;
//! * any sequence of `std::fs` calls. `tokio::fs` is itself `spawn_blocking`
//!   underneath, so one explicit `run_blocking` around a whole sequence of
//!   file operations is cheaper than many implicit ones.
//!
//! # What does not belong here
//!
//! Anything touching the database. `sqlx` is async-native and a pool
//! connection must never be held across this call — the closure runs on a
//! different thread for an unbounded time, and holding a connection for its
//! duration drains the pool (D8). Acquire late, release early: do the query,
//! `await` it, *then* hand the resulting bytes to `run_blocking`.
//!
//! # Why this is not just `spawn_blocking`
//!
//! Two reasons the wrapper earns its keep. It converts a panic into an
//! [`AppError::Internal`] — a bare `spawn_blocking` handle yields a
//! `JoinError` that is easy to `unwrap`, and a panic in a decoder on
//! attacker-supplied bytes should be a 500, not a process abort. And it gives
//! the convention a single grep-able name, so "is anything CPU-bound running
//! on a worker thread?" is answerable by reading call sites rather than by
//! reading every handler.
//!
//! # Bounding the pool
//!
//! `spawn_blocking` has its own thread pool (512 threads by default) and its
//! queue is unbounded. That is fine for the current call volume, but it is not
//! a concurrency limit: fifty simultaneous decodes will happily run fifty
//! threads and thrash the CPU. When phase 4 lands, the parallelism wants
//! shaping — a semaphore around these calls, or a Rayon pool sized to the core
//! count. Noted here rather than built now, because the right bound is a
//! measurement, not a guess.

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
    // The outer `Result` is the join result: `Err` here means the closure
    // panicked (a blocking task cannot be cancelled, so `JoinError` is always
    // a panic in practice). Flatten it into the caller's error type so a panic
    // is a 500 with the real cause on the `#[source]` chain, not a lost thread.
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(AppError::from_join(join_err)),
    }
}

/// Runs infallible CPU-bound or blocking work on the blocking thread pool.
///
/// The separate entry point exists because the fallible form cannot express
/// "this cannot fail" without an awkward turbofish — a closure ending in
/// `Ok(hash)` leaves the error type unconstrained, so every hashing call site
/// would have to write `Ok::<_, AppError>(hash)`. Content hashing and resizing
/// an already-decoded image are the phase 4 cases.
///
/// The returned `AppResult` is still fallible: a panic in `f` is the only way
/// this returns `Err`.
pub async fn run_blocking_infallible<F, T>(f: F) -> AppResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
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
mod tests {
    use super::*;

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
}
