//! Exercise the deployed operator binary, including exit status and unchanged rows.
use sqlx::{ConnectOptions as _, PgPool};
use std::process::Command;
use uuid::Uuid;

fn command(pool: &PgPool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pinvoyager-admin"));
    for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("PINVOYAGER__")) {
        command.env_remove(key);
    }
    command.env(
        "PINVOYAGER__DATABASE__URL",
        pool.connect_options().to_url_lossy().as_str(),
    );
    command
}

#[test]
fn rejects_invalid_arguments_and_supports_help() {
    for args in [
        vec![],
        vec!["bad-uuid"],
        vec!["--initial-admin-email", "a@b"],
        vec!["--help", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-admin"))
            .args(args)
            .output()
            .expect("run");
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
    for binary in [
        env!("CARGO_BIN_EXE_pinvoyager-admin"),
        env!("CARGO_BIN_EXE_pinvoyager-api"),
    ] {
        for flag in ["--help", "-h"] {
            let output = Command::new(binary).arg(flag).output().expect("help");
            assert!(output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-admin"))
            .arg(std::ffi::OsString::from_vec(vec![0xff]))
            .output()
            .expect("non-UTF-8 UUID");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("UTF-8"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-admin"))
        .env("PINVOYAGER__SERVER__BIND", "invalid")
        .arg(Uuid::now_v7().to_string())
        .output()
        .expect("invalid config");
    assert!(!output.status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-api"))
        .args(["--initial-admin-email", "a@b"])
        .output()
        .expect("removed flag");
    assert!(!output.status.success());
}

#[sqlx::test]
async fn grants_existing_accounts_and_reports_no_op_and_missing_users(pool: PgPool) {
    let missing = Uuid::now_v7();
    let output = command(&pool)
        .arg(missing.to_string())
        .output()
        .expect("missing");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
    for suspended in [false, true] {
        let user = pinvoyager_api::db::repo::users::create(&pool, None, None)
            .await
            .expect("account");
        sqlx::query("UPDATE users SET suspended=$2 WHERE id=$1")
            .bind(user.id)
            .bind(suspended)
            .execute(&pool)
            .await
            .expect("status");
        let output = command(&pool)
            .arg(user.id.to_string())
            .output()
            .expect("grant");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("granted"));
        let before: (bool, bool, chrono::DateTime<chrono::Utc>) =
            sqlx::query_as("SELECT is_site_admin,suspended,updated_at FROM users WHERE id=$1")
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .expect("role");
        assert_eq!((before.0, before.1), (true, suspended));
        let output = command(&pool)
            .arg(user.id.to_string())
            .output()
            .expect("no op");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("already"));
        let after: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT updated_at FROM users WHERE id=$1")
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .expect("unchanged");
        assert_eq!(before.2, after);
    }
    let output = command(&pool)
        .env(
            "PINVOYAGER__DATABASE__URL",
            "postgres://nobody@127.0.0.1:1/none",
        )
        .env("PINVOYAGER__DATABASE__ACQUIRE_TIMEOUT_SECS", "1")
        .arg(missing.to_string())
        .output()
        .expect("database failure");
    assert!(!output.status.success());
}
