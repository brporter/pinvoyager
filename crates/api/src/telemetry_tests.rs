use super::*;

#[tokio::test]
async fn filter_and_subscriber_setup_handle_both_formats_and_errors() {
    // Construct each formatter so the JSON and pretty branches are both
    // exercised independently of which one the global subscriber installs.
    let _pretty = fmt_layer::<tracing_subscriber::Registry>(false);
    let _json = fmt_layer::<tracing_subscriber::Registry>(true);

    let config = TelemetryConfig {
        log_level: "info".to_owned(),
        json: true,
    };
    assert!(
        init_with_directives(&config, Some("=info")).is_err(),
        "invalid RUST_LOG must be reported"
    );

    init_with_directives(&config, None).expect("config filter should install the subscriber");
    assert!(
        init_with_directives(&config, Some("info")).is_err(),
        "a second global subscriber is an error"
    );
    // Public entrypoint reads the process environment and reports the already
    // installed global subscriber instead of panicking.
    assert!(init(&config).is_err());
}
