//! HTTP layer.
//!

pub(crate) mod account_deletions;
pub mod admin;
pub mod auth;
pub(crate) mod downloads;
pub mod health;
pub mod middleware;
pub(crate) mod moderation;
pub(crate) mod mutations;
pub mod photos;
pub mod profiles;
pub(crate) mod publication;
pub(crate) mod uploads;
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
        .merge(admin::routes())
        .merge(account_deletions::routes())
        .merge(downloads::routes())
        .merge(moderation::routes())
        .merge(photos::routes())
        .merge(profiles::routes())
        .merge(voyages::routes())
        .merge(mutations::routes())
        .merge(uploads::routes())
        .merge(publication::routes())
        .with_state(state);

    middleware::apply(routes, &config.server)
}

/// Serves until SIGINT or SIGTERM, then waits for active requests up to the
/// configured timeout. Remaining connection tasks and handlers are canceled
/// and joined before worker shutdown begins.
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

    // Poll once before exposing readiness: async signal registration otherwise
    // starts after the log, when an immediate SIGTERM can still kill the process.
    let mut shutdown = Box::pin(shutdown);
    let already_stopped = std::future::poll_fn(|context| {
        std::task::Poll::Ready(shutdown.as_mut().poll(context).is_ready())
    })
    .await;
    let shutdown = async move {
        if !already_stopped {
            shutdown.await;
        }
    };
    let listener = TcpListener::bind(bind).await?;
    // Log the resolved address rather than the configured one: binding port 0
    // is how the integration tests get an ephemeral port, and "listening on
    // 0" is useless.
    tracing::info!(addr = %listener.local_addr()?, "listening");

    serve_router(listener, router(state), shutdown, drain_deadline).await
}

/// Listener and router injection lets drain tests own a deliberately pending
/// request without timing races or changes to the production route surface.
async fn serve_router<F>(
    mut listener: TcpListener,
    routes: Router,
    shutdown: F,
    drain_deadline: Duration,
) -> Result<(), io::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    // Own HTTP/1 connection tasks so the deadline can cancel handlers and
    // response streams. This preserves the protocol enabled by our axum build.
    let mut connections = tokio::task::JoinSet::new();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            _ = connections.join_next(), if !connections.is_empty() => {},
            (socket, _) = axum::serve::Listener::accept(&mut listener) => {
                let service = hyper_util::service::TowerToHyperService::new(routes.clone());
                let mut stopped = stopped.clone();
                connections.spawn(async move {
                    let builder = hyper::server::conn::http1::Builder::new();
                    let connection = builder.serve_connection(hyper_util::rt::TokioIo::new(socket), service);
                    tokio::pin!(connection);
                    let result = tokio::select! {
                        result = &mut connection => result,
                        _ = stopped.changed() => {
                            connection.as_mut().graceful_shutdown();
                            connection.await
                        }
                    };
                    if let Err(error) = result {
                        tracing::debug!(%error, "HTTP connection ended with an error");
                    }
                });
            }
        }
    }
    drop(listener);
    let _ = stop.send(true);
    tokio::select! {
        () = async { while connections.join_next().await.is_some() {} } => {},
        () = tokio::time::sleep(drain_deadline) => {
            tracing::warn!("shutdown deadline exceeded; canceling remaining HTTP connections");
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        }
    }
    Ok(())
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
