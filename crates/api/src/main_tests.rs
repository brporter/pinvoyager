use super::*;
use sqlx::ConnectOptions as _;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// Each child has its own environment, telemetry subscriber, and signal handlers.
fn child(database_url: &str, bind: &str, expected: &str, dev_login: bool) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    for (key, _) in
        std::env::vars().filter(|(key, _)| key.starts_with("PINVOYAGER__") || key == "RUST_LOG")
    {
        command.env_remove(key);
    }
    command
        .args(["--exact", "tests::entrypoint_child", "--nocapture"])
        .env("PINVOYAGER_ENTRY_CHILD_EXPECT", expected)
        .env("PINVOYAGER__DATABASE__URL", database_url)
        .env("PINVOYAGER__SERVER__BIND", bind)
        .env("PINVOYAGER__SERVER__SHUTDOWN_TIMEOUT_SECS", "2")
        .env("PINVOYAGER__DATABASE__ACQUIRE_TIMEOUT_SECS", "1")
        .env("PINVOYAGER__AUTH__DEV_LOGIN", dev_login.to_string())
        .env("PINVOYAGER__TELEMETRY__JSON", "true")
        .env("PINVOYAGER__TELEMETRY__LOG_LEVEL", "trace")
        .env("PINVOYAGER__TELEMETRY__OTLP_ENDPOINT", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[test]
fn entrypoint_child() {
    let Ok(expected) = std::env::var("PINVOYAGER_ENTRY_CHILD_EXPECT") else {
        return;
    };
    let actual = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(entrypoint([]));
    assert_eq!(
        actual,
        if expected == "success" {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    );
}

#[sqlx::test]
async fn startup_and_shutdown_complete_in_an_isolated_process(pool: sqlx::PgPool) {
    let url = pool.connect_options().to_url_lossy().to_string();
    for dev_login in [true, false] {
        let mut child = child(&url, "127.0.0.1:0", "success", dev_login)
            .spawn()
            .expect("start child");
        let mut output = BufReader::new(child.stdout.take().expect("stdout"));
        let ready = (&mut output)
            .lines()
            .map(|line| line.expect("read readiness"))
            .any(|line| line.contains("listening"));
        assert!(ready, "child did not start listening");
        assert!(
            Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .expect("signal child")
                .success()
        );
        // Continue draining output while the child exits; trace logs must not
        // fill a pipe or make successful shutdown depend on output volume.
        for line in output.lines() {
            line.expect("shutdown output");
        }
        let result = child.wait_with_output().expect("child exits");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[sqlx::test]
async fn startup_failure_reports_its_causes(pool: sqlx::PgPool) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let url = pool.connect_options().to_url_lossy().to_string();
    let result = child(
        &url,
        &listener.local_addr().expect("address").to_string(),
        "failure",
        false,
    )
    .output()
    .expect("child result");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("fatal:"));

    let failed = child(
        "postgres://nobody@127.0.0.1:1/none",
        "127.0.0.1:0",
        "failure",
        false,
    )
    .output()
    .expect("failed startup result");
    assert!(failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("caused by:"));
}

#[test]
fn invalid_startup_configuration_is_reported_before_database_access() {
    for (key, value) in [
        ("PINVOYAGER__SERVER__BIND", "not a socket address"),
        ("PINVOYAGER__TELEMETRY__LOG_LEVEL", "=info"),
    ] {
        let output = child(
            "postgres://nobody@127.0.0.1:1/none",
            "127.0.0.1:0",
            "failure",
            false,
        )
        .env(key, value)
        .output()
        .expect("child result");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("fatal:"));
    }
}
