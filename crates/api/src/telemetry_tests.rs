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
        ..TelemetryConfig::default()
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
    let exporter_config = TelemetryConfig {
        otlp_endpoint: Some("http://127.0.0.1:4318/v1/traces".to_owned()),
        ..TelemetryConfig::default()
    };
    assert!(init_with_directives(&exporter_config, None).is_err());
}

#[test]
fn w3c_context_accepts_valid_headers_and_ignores_missing_or_invalid_ones() {
    use opentelemetry::trace::TraceContextExt;
    let mut headers = HeaderMap::new();
    assert!(!extract_context(&headers).span().span_context().is_valid());
    headers.insert("traceparent", HeaderValue::from_static("invalid"));
    assert!(!extract_context(&headers).span().span_context().is_valid());
    headers.insert(
        "traceparent",
        HeaderValue::from_bytes(&[0xff]).expect("opaque header"),
    );
    assert!(!extract_context(&headers).span().span_context().is_valid());
    headers.insert(
        "traceparent",
        HeaderValue::from_static("00-11111111111111111111111111111111-2222222222222222-01"),
    );
    headers.insert("tracestate", HeaderValue::from_static("vendor=value"));
    let context = extract_context(&headers);
    assert!(context.span().span_context().is_remote());
    assert!(context.span().span_context().is_sampled());
    assert_eq!(Headers(&headers).keys().len(), 2);
    let mut outgoing = HeaderMap::new();
    inject_context(&context, &mut outgoing);
    assert_eq!(outgoing.get("traceparent"), headers.get("traceparent"));
    assert_eq!(outgoing.get("tracestate"), headers.get("tracestate"));
    ResponseHeaders(&mut outgoing).set("invalid\nname", "ignored".to_owned());
    ResponseHeaders(&mut outgoing).set("valid", "invalid\nvalue".to_owned());
    assert_eq!(outgoing.len(), 2);
}

#[tokio::test]
async fn optional_exporter_builds_shuts_down_and_reports_invalid_endpoints() {
    let mut config = TelemetryConfig::default();
    assert!(build_provider(&config).expect("disabled").is_none());
    shutdown(None).await;
    config.otlp_endpoint = Some(String::new());
    assert!(
        build_provider(&config)
            .expect("empty endpoint disables export")
            .is_none()
    );
    config.otlp_endpoint = Some("not a URL".to_owned());
    assert!(build_provider(&config).is_err());
    config.otlp_endpoint = Some("http://127.0.0.1:4318/v1/traces".to_owned());
    let provider = build_provider(&config).expect("exporter");
    provider
        .as_ref()
        .expect("enabled")
        .shutdown()
        .expect("first shutdown");
    shutdown(provider).await; // Already shut down: log and continue rather than panic.
    shutdown(build_provider(&config).expect("exporter")).await;
}
