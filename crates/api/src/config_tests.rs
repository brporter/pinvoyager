use super::*;

#[test]
fn serde_applies_defaults_to_optional_configuration_sections() {
    let server: ServerConfig = serde_json::from_str("{}").expect("server defaults");
    assert_eq!(server.bind, "127.0.0.1:8080".parse().unwrap());
    assert_eq!(server.shutdown_timeout_secs, 30);
    assert_eq!(server.request_timeout_secs, 30);

    let photos: PhotosConfig = serde_json::from_str("{}").expect("photo defaults");
    assert_eq!(photos.max_upload_bytes, 25 * 1024 * 1024);
    assert_eq!(photos.max_megapixels, 100);
    assert_eq!(photos.upload_rate_per_minute, 60);
    assert_eq!(photos.variant_queue_depth, 256);
    assert_eq!(photos.variant_concurrency, 4);
    assert_eq!(photos.webp_quality, 82.0);

    let session: SessionConfig = serde_json::from_str("{}").expect("session defaults");
    assert_eq!(session.idle_timeout_secs, 14 * 24 * 60 * 60);
    assert_eq!(session.absolute_timeout_secs, 90 * 24 * 60 * 60);
    assert_eq!(session.cookie_name, "pv_session");
    assert!(session.cookie_secure);

    let telemetry: TelemetryConfig = serde_json::from_str("{}").expect("telemetry defaults");
    assert!(!telemetry.json);
    assert!(telemetry.log_level.contains("pinvoyager_api=debug"));

    let providers: ProvidersConfig = serde_json::from_str("{}").expect("provider defaults");
    assert!(providers.google.is_none());
    assert!(providers.microsoft.is_none());
    assert!(providers.apple.is_none());
    assert!(providers.facebook.is_none());
}

#[test]
fn load_reads_the_repository_configuration_and_default_values() {
    let config = Config::load().expect("repository config is valid");

    assert_eq!(config.database.max_connections, DEFAULT_MAX_CONNECTIONS);
    assert_eq!(config.database.min_connections, DEFAULT_MIN_CONNECTIONS);
    assert_eq!(
        config.database.acquire_timeout_secs,
        DEFAULT_ACQUIRE_TIMEOUT_SECS
    );
    assert_eq!(config.server.bind, "127.0.0.1:8080".parse().unwrap());
    assert!(config.auth.dev_login);
}

#[test]
fn figment_errors_name_missing_settings_and_both_configuration_sources() {
    let error = Figment::new()
        .merge(Serialized::default(
            "database.max_connections",
            DEFAULT_MAX_CONNECTIONS,
        ))
        .extract::<Config>()
        .expect_err("required database and auth settings are missing");
    let message = ConfigError::from_figment(error).to_string();

    assert!(message.contains("database.url"), "{message}");
    assert!(message.contains("PINVOYAGER__DATABASE__URL"), "{message}");
}

#[test]
fn describe_handles_missing_fields_and_errors_without_a_key_path() {
    let no_path = figment::Error::from(figment::error::Kind::Message("invalid input".into()));
    assert_eq!(describe(no_path), "invalid input");

    let missing = figment::Error::from(figment::error::Kind::MissingField("url".into()));
    let description = describe(missing);
    assert!(description.contains("url"), "{description}");
    assert!(description.contains("PINVOYAGER__URL"), "{description}");

    let mut malformed = figment::Error::from(figment::error::Kind::Message("invalid type".into()));
    malformed.path.push("telemetry".into());
    let description = describe(malformed);
    assert!(description.contains("`telemetry`"), "{description}");
    assert!(
        description.contains("PINVOYAGER__TELEMETRY"),
        "{description}"
    );
}
