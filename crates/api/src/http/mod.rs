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
        shutdown_signal().await;
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
            tracing::warn!(
                timeout_secs = drain_deadline.as_secs(),
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
        match tokio::signal::ctrl_c().await {
            Ok(()) => tracing::info!("received SIGINT"),
            Err(err) => {
                tracing::error!(error = %err, "could not listen for SIGINT");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
                tracing::info!("received SIGTERM");
            }
            Err(err) => {
                tracing::error!(error = %err, "could not listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
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
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use secrecy::SecretString;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt as _;

    use super::*;
    use crate::config::{Config, DatabaseConfig, ServerConfig, TelemetryConfig};

    fn test_config() -> Config {
        {
            Config {
                server: ServerConfig {
                    bind: "127.0.0.1:0".parse().expect("valid socket addr"),
                    shutdown_timeout_secs: 5,
                    request_timeout_secs: 5,
                },
                database: DatabaseConfig {
                    url: SecretString::from("postgres://nobody@127.0.0.1:1/none"),
                    max_connections: 1,
                    min_connections: 0,
                    acquire_timeout_secs: 1,
                },
                telemetry: TelemetryConfig {
                    log_level: "info".to_owned(),
                    json: false,
                },
                auth: crate::config::AuthConfig {
                    base_url: "http://127.0.0.1:8080".parse().expect("valid url"),
                    session: Default::default(),
                    providers: Default::default(),
                    dev_login: false,
                },
                photos: crate::config::PhotosConfig::default(),
            }
        }
    }

    /// A state whose pool is lazy and never connected. Enough to exercise the
    /// router, and deliberately unable to reach a database — which is exactly
    /// what the liveness contract is about.
    fn test_state() -> AppState {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_secs(2))
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .expect("a syntactically valid url makes a lazy pool");

        AppState::new(
            pool,
            std::sync::Arc::new(test_config()),
            // No providers: these tests exercise the router, not sign-in.
            crate::auth::ProviderRegistry::empty(),
            crate::photos::worker::VariantQueue::detached(),
        )
    }

    /// T8's first half, and the reason `/health` exists: no database, still 200.
    #[tokio::test]
    async fn health_is_200_without_a_database() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible");

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// T8's second half: the same process, the same moment, and `/ready` says
    /// no — because it is the endpoint whose answer depends on the database.
    #[tokio::test]
    async fn ready_is_503_without_a_database() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body is small");
        let body = String::from_utf8_lossy(&body);
        // The connection error names the host, the port and the database. None
        // of that belongs in a response that anyone can curl.
        assert!(
            !body.contains("127.0.0.1"),
            "leaked connection detail: {body}"
        );
        assert!(
            !body.contains("refused"),
            "leaked connection detail: {body}"
        );
    }

    /// The "Reference 7f3a-2b91" contract: the id reaches the client on every
    /// response, so a user can quote it.
    #[tokio::test]
    async fn responses_carry_a_request_id() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible");

        let id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .expect("x-request-id is set on every response");

        let parsed = uuid::Uuid::parse_str(id).expect("request id is a uuid");
        assert_eq!(parsed.get_version_num(), 7, "request ids are uuid v7");
    }

    /// The deadline must not start before the signal does, or a long-running
    /// server would shoot itself after `shutdown_timeout_secs` of uptime.
    ///
    /// Wall-clock durations rather than a paused clock, because tokio's
    /// `test-util` feature is not enabled; they are kept small.
    #[tokio::test]
    async fn drain_deadline_does_not_start_before_the_signal() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let deadline = drain_timeout(rx, Duration::from_millis(50));

        tokio::pin!(deadline);
        let early = tokio::time::timeout(Duration::from_millis(200), &mut deadline).await;
        assert!(early.is_err(), "deadline fired without a shutdown signal");

        tx.send(()).expect("receiver is alive");
        tokio::time::timeout(Duration::from_secs(5), deadline)
            .await
            .expect("deadline fires once the signal has been observed");
    }

    /// If the server stops for its own reasons the sender is dropped; the
    /// deadline must then never fire, or it would race the server's result.
    #[tokio::test]
    async fn drain_deadline_never_fires_if_the_sender_is_dropped() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        drop(tx);

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            drain_timeout(rx, Duration::ZERO),
        )
        .await;
        assert!(
            result.is_err(),
            "deadline fired after the sender was dropped"
        );
    }

    /// Real SIGTERM, real listener — the signal a container runtime actually
    /// sends. Ignored by default because the signal goes to the whole test
    /// process: run it on its own with
    /// `cargo test -- --ignored sigterm --test-threads=1`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "sends SIGTERM to the test process"]
    async fn sigterm_stops_the_server() {
        let server = tokio::spawn(serve(test_state()));
        // Give the listener and the signal handler time to be installed; if
        // the handler is missing, the default disposition kills this process,
        // which is a loud enough failure.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let status = std::process::Command::new("kill")
            .args(["-TERM", &std::process::id().to_string()])
            .status()
            .expect("kill(1) runs");
        assert!(status.success());

        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("server returns after SIGTERM")
            .expect("server task did not panic")
            .expect("server returned cleanly");
    }

    /// A route that does not exist still gets an id, because the id layer is
    /// outermost — a 404 is exactly the kind of response a user reports.
    #[tokio::test]
    async fn unmatched_routes_still_get_a_request_id() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/nope")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router is infallible");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(response.headers().contains_key("x-request-id"));
    }
}
