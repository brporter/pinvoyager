use super::*;
fn parse(args: &[&str]) -> Result<Option<InitialAdmins>, String> {
    InitialAdmins::parse_args(args.iter().map(OsString::from))
}
#[test]
fn repeated_values_are_normalized_and_deduplicated_without_alias_expansion() {
    let admins = parse(&[
        "--initial-admin-email",
        " Admin@GMAIL.com ",
        "--initial-admin-email=second@example.com",
        "--initial-admin-email",
        "Admin@gmail.com",
    ])
    .expect("parse")
    .expect("run");
    assert!(admins.matches("Admin@gmail.com"));
    assert!(!admins.matches("admin@gmail.com"));
    assert!(admins.matches("second@example.com"));
    assert!(!admins.matches("admin+alias@gmail.com"));
    assert!(!admins.matches("a.dmin@gmail.com"));
    assert!(!admins.is_empty());
    assert_eq!(admins.emails.len(), 2);
    assert!(!format!("{admins:?}").contains("gmail.com"));
    assert!(!admins.matches("invalid"));
    assert!(parse(&[]).expect("empty").expect("run").is_empty());
    for flag in ["--help", "-h"] {
        assert!(parse(&[flag]).expect("help").is_none());
    }
}
#[test]
fn invalid_arguments_fail_without_echoing_addresses() {
    for args in [
        vec!["--initial-admin-email"],
        vec!["--initial-admin-email="],
        vec!["--unknown"],
        vec!["--initial-admin-email", "no-at"],
        vec!["--initial-admin-email", "a@@b"],
        vec!["--initial-admin-email", "@b"],
        vec!["--initial-admin-email", "a@"],
        vec!["--initial-admin-email", "a b@c"],
        vec!["--initial-admin-email", "é@x"],
        vec!["--initial-admin-email", "a@b\nc"],
    ] {
        assert!(
            parse(&args)
                .expect_err("reject")
                .contains("initial-admin-email")
        );
    }
    use std::os::unix::ffi::OsStringExt;
    assert!(InitialAdmins::parse_args([OsString::from_vec(vec![0xff])]).is_err());
    assert!(
        InitialAdmins::parse_args([
            OsString::from("--initial-admin-email"),
            OsString::from_vec(vec![0xff])
        ])
        .is_err()
    );
}
