//! HTTP layer.
//!
//! CONTRACT — `router` and `serve` signatures are fixed; implementations and
//! everything under `health` / `middleware` are owned by the http agent.

pub mod auth;
pub mod health;
pub mod middleware;
pub mod photos;
pub mod voyages;

use std::time::Duration;
use std::{fmt::Display, future::pending};
use std::{future::Future, io};

use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;

use crate::state::AppState;

/// Builds the router with all middleware applied.
///
/// Separate from [`serve`] so integration tests can drive the router without
/// binding a port.
pub fn router(state: AppState) -> Router {
    // Cloned out before `with_state` consumes the state, because the
    // middleware stack is configured from the same config the handlers see.
    let config = state.config.clone();

    let routes = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .merge(auth::routes())
        .merge(photos::routes())
        .merge(voyages::routes())
        .with_state(state);

    middleware::apply(routes, &config.server)
}

/// Binds the listener and serves until a shutdown signal, then drains.
///
/// Graceful shutdown means in-flight requests finish. A hard drop mid-upload
/// is precisely the failure the design's "your queued photos are still here"
/// message is trying to avoid having to show.
pub async fn serve(state: AppState) -> Result<(), std::io::Error> {
    serve_with_shutdown(state, shutdown_signal()).await
}

/// Runs the server with the supplied shutdown future.
///
/// The production entry point uses the OS signal handler. Keeping the future
/// injectable also lets tests verify listener and drain behavior without
/// sending process-wide signals.
async fn serve_with_shutdown<F>(state: AppState, shutdown: F) -> Result<(), io::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    let bind = state.config.server.bind;
    let drain_deadline = Duration::from_secs(state.config.server.shutdown_timeout_secs);

    let listener = TcpListener::bind(bind).await?;
    // Log the resolved address rather than the configured one: binding port 0
    // is how the integration tests get an ephemeral port, and "listening on
    // 0" is useless.
    tracing::info!(addr = %listener.local_addr()?, "listening");

    // `shutdown_started` fires the moment a signal arrives, which is *before*
    // draining finishes. It is what starts the drain clock below.
    let (shutdown_started, drain_clock) = tokio::sync::oneshot::channel::<()>();

    let server = axum::serve(listener, router(state)).with_graceful_shutdown(async move {
        shutdown.await;
        // The receiver is only dropped if the server already stopped, in which
        // case nobody is waiting on the deadline.
        let _ = shutdown_started.send(());
    });

    tokio::select! {
        // Normal path: every in-flight request finished and the server
        // returned on its own.
        result = server => result,

        // Backstop: `axum::serve` waits for in-flight requests *indefinitely*,
        // so one wedged handler would keep the process alive forever — and a
        // container runtime that sent SIGTERM will SIGKILL it after its own
        // grace period, which is a worse ending than this one. Dropping the
        // server future here closes the remaining connections.
        () = drain_timeout(drain_clock, drain_deadline) => {
            let timeout_secs = drain_deadline.as_secs();
            tracing::warn!(
                timeout_secs,
                "shutdown drain deadline exceeded; dropping remaining connections"
            );
            Ok(())
        }
    }
}

/// Resolves `deadline` after a shutdown signal is observed, and never before.
async fn drain_timeout(started: tokio::sync::oneshot::Receiver<()>, deadline: Duration) {
    if started.await.is_err() {
        // The sender was dropped without signaling: the server finished for
        // some other reason. Never resolve, so this branch cannot win the
        // race and swallow the server's own result.
        std::future::pending::<()>().await;
    }
    tokio::time::sleep(deadline).await;
}

#[derive(Clone, Copy)]
enum ShutdownSignalKind {
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

async fn observe_signal<T, E, Fut>(
    setup: Result<T, E>,
    signal_kind: ShutdownSignalKind,
    wait: impl FnOnce(T) -> Fut,
) where
    E: Display,
    Fut: Future<Output = ()>,
{
    let signal = match setup {
        Ok(signal) => signal,
        Err(err) => {
            let error_message = err.to_string();
            match signal_kind {
                ShutdownSignalKind::Interrupt => {
                    tracing::error!(error = %error_message, "could not listen for SIGINT")
                }
                #[cfg(unix)]
                ShutdownSignalKind::Terminate => {
                    tracing::error!(error = %error_message, "could not listen for SIGTERM")
                }
            }
            pending::<T>().await
        }
    };

    wait(signal).await;
    match signal_kind {
        ShutdownSignalKind::Interrupt => tracing::info!("received SIGINT"),
        #[cfg(unix)]
        ShutdownSignalKind::Terminate => tracing::info!("received SIGTERM"),
    }
}

/// Completes on SIGINT or SIGTERM.
///
/// SIGTERM is the one that matters in a container: it is what every runtime
/// sends first, and a process that only handles Ctrl-C gets SIGKILLed at the
/// end of the grace period with its in-flight requests still open.
async fn shutdown_signal() {
    let interrupt = async {
        // `ctrl_c` failing means the OS refused to install a handler; there is
        // no sane recovery, and returning would trigger an immediate shutdown,
        // so wait forever and let SIGTERM do the work.
        observe_signal(
            tokio::signal::ctrl_c().await,
            ShutdownSignalKind::Interrupt,
            |()| async {},
        )
        .await;
    };

    #[cfg(unix)]
    let terminate = async {
        let signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        observe_signal(
            signal,
            ShutdownSignalKind::Terminate,
            |mut stream| async move {
                stream.recv().await;
            },
        )
        .await;
    };

    // Windows has no SIGTERM; Ctrl-C is the whole story there.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received; draining in-flight requests");
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
