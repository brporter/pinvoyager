use std::process::Command;

#[test]
fn initial_admin_arguments_are_validated_before_server_startup() {
    for args in [
        vec!["--initial-admin-email"],
        vec!["--initial-admin-email", "invalid"],
        vec!["--unknown-option"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-api"))
            .args(args)
            .env("PINVOYAGER__SERVER__BIND", "not an address")
            .output()
            .expect("start API binary");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("initial-admin-email"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-api"))
        .arg("--help")
        .env("PINVOYAGER__SERVER__BIND", "not an address")
        .output()
        .expect("API help");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--initial-admin-email"));
}

#[test]
fn repeated_valid_addresses_reach_configuration_loading() {
    let output = Command::new(env!("CARGO_BIN_EXE_pinvoyager-api"))
        .args([
            "--initial-admin-email",
            "first@gmail.com",
            "--initial-admin-email=second@example.com",
        ])
        .env("PINVOYAGER__SERVER__BIND", "not an address")
        .output()
        .expect("API process");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("initial-admin-email"));
    assert!(stderr.contains("bind"));
    assert!(!stderr.contains("first@gmail.com"));
}
