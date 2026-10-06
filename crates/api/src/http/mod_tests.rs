use axum::body::Body;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use secrecy::SecretString;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::io::AsyncWriteExt as _;
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

fn test_state_with_pool(pool: PgPool) -> AppState {
    test_state_with_config(pool, test_config())
}

fn test_state_with_config(pool: PgPool, config: crate::config::Config) -> AppState {
    AppState::new(
        pool,
        std::sync::Arc::new(config),
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

#[sqlx::test]
async fn ready_is_200_when_the_database_answers(pool: PgPool) {
    let response = router(test_state_with_pool(pool))
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("router is infallible");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096)
        .await
        .expect("response body is small");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["status"],
        "ready"
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

#[tokio::test]
async fn signal_setup_failures_remain_pending_for_each_supported_signal() {
    let interrupt_error: std::io::Result<()> = Err(std::io::Error::other("no SIGINT handler"));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(10),
            observe_signal(interrupt_error, ShutdownSignalKind::Interrupt, |()| async {
            },),
        )
        .await
        .is_err()
    );

    #[cfg(unix)]
    {
        let terminate_error: std::io::Result<tokio::signal::unix::Signal> =
            Err(std::io::Error::other("no SIGTERM handler"));
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                observe_signal(terminate_error, ShutdownSignalKind::Terminate, |_| async {}),
            )
            .await
            .is_err()
        );
    }
}

/// Real SIGTERM, real listener — the signal a container runtime actually
/// sends. The coverage command runs the suite serially so this process-wide
/// signal cannot interrupt another test.
#[tokio::test(flavor = "multi_thread")]
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

/// SIGINT exercises the other shutdown signal used for local terminal runs.
#[tokio::test(flavor = "multi_thread")]
async fn sigint_stops_the_server() {
    let server = tokio::spawn(serve(test_state()));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let status = std::process::Command::new("kill")
        .args(["-INT", &std::process::id().to_string()])
        .status()
        .expect("kill(1) runs");
    assert!(status.success());

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server returns after SIGINT")
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

#[tokio::test]
async fn serve_returns_an_error_when_the_configured_address_is_unavailable() {
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    let mut config = test_config();
    config.server.bind = occupied.local_addr().expect("read reserved address");
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("valid lazy pool URL");
    let state = AppState::new(
        pool,
        std::sync::Arc::new(config),
        crate::auth::ProviderRegistry::empty(),
        crate::photos::worker::VariantQueue::detached(),
    );

    assert!(serve(state).await.is_err());
}

#[tokio::test]
async fn serve_returns_cleanly_after_a_shutdown_signal_with_no_requests_in_flight() {
    serve_with_shutdown(test_state(), async {})
        .await
        .expect("a clean graceful shutdown returns success");
}

#[sqlx::test]
async fn server_drops_a_stalled_request_when_the_drain_deadline_expires(pool: PgPool) {
    let user = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("create request owner");
    let mut config = test_config();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    config.server.bind = reservation.local_addr().unwrap();
    config.server.shutdown_timeout_secs = 0;
    drop(reservation);
    let bind = config.server.bind;
    let token = crate::auth::session::create(&pool, user.id, &config.auth.session)
        .await
        .expect("create request session");
    let cookie = format!("{}={}", config.auth.session.cookie_name, token.expose());
    let state = test_state_with_config(pool, config);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_with_shutdown(state, async move {
        let _ = shutdown_rx.await;
    }));

    let mut socket = loop {
        match tokio::net::TcpStream::connect(bind).await {
            Ok(socket) => break socket,
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    };
    socket
        .write_all(
            format!(
                "POST /api/photos HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nContent-Type: multipart/form-data; boundary=pv\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("send a partial chunked request");
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown_tx.send(()).expect("shutdown receiver is alive");

    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("drain deadline releases the stalled request")
        .expect("server task did not panic")
        .expect("server stops cleanly after dropping the request");
}
