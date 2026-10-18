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
                ..TelemetryConfig::default()
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
        serde_json::from_slice::<serde_json::Value>(&body).expect("valid test fixture")["status"],
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

/// Actual signals run in a child process; readiness follows handler registration.
#[test]
fn operating_system_signals_stop_a_child_server() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    for signal in ["-TERM", "-INT"] {
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "http::tests::signal_child", "--nocapture"])
            .env("PINVOYAGER_SIGNAL_CHILD", "1")
            .stdout(Stdio::piped())
            .spawn()
            .expect("child server");
        let stdout = child.stdout.take().expect("child stdout");
        let mut output = BufReader::new(stdout);
        let ready = (&mut output)
            .lines()
            .map(|line| line.expect("read readiness"))
            .any(|line| line == "SIGNALS_READY");
        assert!(
            ready,
            "child must install handlers before receiving a signal"
        );
        assert!(
            Command::new("kill")
                .args([signal, &child.id().to_string()])
                .status()
                .expect("send signal")
                .success()
        );
        assert!(child.wait().expect("wait for child").success());
    }
}

#[tokio::test]
async fn signal_child() {
    if std::env::var_os("PINVOYAGER_SIGNAL_CHILD").is_none() {
        return;
    }
    let mut shutdown = Box::pin(shutdown_signal());
    tokio::select! {
        biased;
        () = &mut shutdown => panic!("unexpected signal before readiness"),
        () = tokio::task::yield_now() => {}
    }
    println!("SIGNALS_READY");
    tokio::time::timeout(
        Duration::from_secs(10),
        serve_with_shutdown(test_state(), shutdown),
    )
    .await
    .expect("child shutdown deadline")
    .expect("server drains");
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
    let voyage_id = crate::photo_voyage::fixture_voyage(&pool, user.id).await;
    let mut config = test_config();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    config.server.bind = reservation.local_addr().expect("valid test fixture");
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
                "POST /api/voyages/{voyage_id}/photos HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nContent-Type: multipart/form-data; boundary=pv\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n"
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

#[tokio::test]
async fn an_inflight_request_cannot_hold_shutdown_past_its_deadline() {
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let route_started = started.clone();
    let dropped = std::sync::Arc::new(tokio::sync::Notify::new());
    struct OnDrop(std::sync::Arc<tokio::sync::Notify>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let route_dropped = dropped.clone();
    let routes = Router::new().route(
        "/blocked",
        axum::routing::get(move || {
            let started = route_started.clone();
            let dropped = route_dropped.clone();
            async move {
                let _guard = OnDrop(dropped);
                started.notify_one();
                std::future::pending::<String>().await
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_router(
        listener,
        routes,
        async {
            stopped.await.expect("shutdown signal");
        },
        Duration::from_millis(5),
    ));
    let request =
        tokio::spawn(async move { reqwest::get(format!("http://{address}/blocked")).await });
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .expect("handler entered");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !server.is_finished(),
        "the deadline must not start before shutdown"
    );
    stop.send(()).expect("signal listener");
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .expect("drain is bounded")
        .expect("server task")
        .expect("server result");
    tokio::time::timeout(Duration::from_secs(1), dropped.notified())
        .await
        .expect("handler is canceled before server returns");
    assert!(request.await.expect("request task").is_err());
}

#[tokio::test]
async fn shutdown_allows_an_active_request_to_finish_before_the_deadline() {
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let finish = std::sync::Arc::new(tokio::sync::Notify::new());
    let route_started = started.clone();
    let route_finish = finish.clone();
    let routes = Router::new().route(
        "/slow",
        get(move || {
            let started = route_started.clone();
            let finish = route_finish.clone();
            async move {
                started.notify_one();
                finish.notified().await;
                "finished"
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_router(
        listener,
        routes,
        async {
            stopped.await.expect("stop");
        },
        Duration::from_secs(5),
    ));
    let request = tokio::spawn(async move {
        reqwest::get(format!("http://{address}/slow"))
            .await
            .expect("response")
            .text()
            .await
            .expect("body")
    });
    started.notified().await;
    stop.send(()).expect("signal");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!server.is_finished());
    assert!(
        tokio::net::TcpStream::connect(address).await.is_err(),
        "listener is closed during draining"
    );
    finish.notify_one();
    assert_eq!(request.await.expect("request"), "finished");
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .expect("server finished")
        .expect("task")
        .expect("server");
}

#[tokio::test]
async fn shutdown_cancels_a_streamed_response_at_the_deadline() {
    let (send, receive) = tokio::sync::mpsc::channel::<Result<&'static str, std::io::Error>>(1);
    send.send(Ok("first chunk")).await.expect("chunk");
    let receive = std::sync::Arc::new(std::sync::Mutex::new(Some(receive)));
    let routes = Router::new().route(
        "/stream",
        get(move || {
            let receive = receive.lock().expect("lock").take().expect("one request");
            async move { Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receive)) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_router(
        listener,
        routes,
        async {
            stopped.await.expect("stop");
        },
        Duration::from_millis(5),
    ));
    let response = reqwest::get(format!("http://{address}/stream"))
        .await
        .expect("headers");
    stop.send(()).expect("signal");
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .expect("deadline")
        .expect("task")
        .expect("server");
    tokio::time::timeout(Duration::from_secs(1), send.closed())
        .await
        .expect("response stream dropped");
    assert!(
        response.bytes().await.is_err(),
        "client observes the truncated stream"
    );
}
