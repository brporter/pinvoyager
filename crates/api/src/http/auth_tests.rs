use super::*;

struct MislabelledProvider;

#[async_trait::async_trait]
impl crate::auth::AuthProvider for MislabelledProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn authorize(&self) -> Result<crate::auth::provider::AuthorizationRedirect, AuthError> {
        unreachable!("the callback test does not start an authorization")
    }

    async fn exchange(
        &self,
        _code: &str,
        _pkce_verifier: &str,
        _nonce: &str,
        _form_body: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        Ok(VerifiedIdentity {
            provider: ProviderKind::Facebook,
            subject: "mislabelled-subject".to_owned(),
            display_name: None,
            avatar_url: None,
        })
    }
}

struct StaticIdentityProvider;

async fn install_identity_collision_trigger(pool: &sqlx::PgPool) {
    sqlx::query(
        "CREATE FUNCTION force_identity_unique_violation() RETURNS trigger
         LANGUAGE plpgsql AS $$
         BEGIN
             RAISE unique_violation USING MESSAGE = 'simulated concurrent identity insert';
         END
         $$",
    )
    .execute(pool)
    .await
    .expect("create collision trigger function");
    sqlx::query(
        "CREATE TRIGGER force_identity_unique_violation
         BEFORE INSERT ON identities
         FOR EACH ROW EXECUTE FUNCTION force_identity_unique_violation()",
    )
    .execute(pool)
    .await
    .expect("create collision trigger");
}

#[async_trait::async_trait]
impl crate::auth::AuthProvider for StaticIdentityProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn authorize(&self) -> Result<crate::auth::provider::AuthorizationRedirect, AuthError> {
        unreachable!("the sign-in callback test does not start an authorization")
    }

    async fn exchange(
        &self,
        _code: &str,
        _pkce_verifier: &str,
        _nonce: &str,
        _form_body: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        Ok(VerifiedIdentity {
            provider: ProviderKind::Google,
            subject: "simulated-concurrent-first-sign-in".to_owned(),
            display_name: Some("No account survives the collision".to_owned()),
            avatar_url: None,
        })
    }
}

#[sqlx::test]
async fn concurrent_first_sign_in_unique_violation_is_a_safe_rejection(pool: sqlx::PgPool) {
    crate::db::migrate(&pool).await.expect("schema is migrated");
    install_identity_collision_trigger(&pool).await;

    let binding = BindingValue::generate();
    let state = "state-concurrent-first-sign-in";
    crate::auth::attempt::create(
        &pool,
        &PendingAttempt {
            state: state.to_owned(),
            provider: ProviderKind::Google,
            nonce: "nonce".to_owned(),
            pkce_verifier: "verifier".to_owned(),
            link_user_id: None,
            redirect_to: None,
            expires_at: Utc::now() + ATTEMPT_TTL,
            binding_hash: binding.hash(),
        },
    )
    .await
    .expect("create pending attempt");

    let config: crate::config::Config = serde_json::from_value(serde_json::json!({
        "database": {
            "url": "postgres://pinvoyager:pinvoyager@localhost/pinvoyager",
            "max_connections": 1,
            "min_connections": 0,
            "acquire_timeout_secs": 2
        },
        "auth": { "base_url": "http://localhost:8080/" }
    }))
    .expect("test config deserializes");
    let app = crate::state::AppState::new(
        pool,
        std::sync::Arc::new(config),
        crate::auth::ProviderRegistry::for_test(
            ProviderKind::Google,
            Box::new(StaticIdentityProvider),
        ),
        crate::photos::worker::VariantQueue::detached(),
    );
    let jar = CookieJar::new().add(
        Cookie::build((BINDING_COOKIE, binding.expose().to_owned()))
            .path("/")
            .build(),
    );

    let result = complete(
        &app,
        ProviderKind::Google,
        jar,
        Callback {
            code: Some("code"),
            state: Some(state),
            error: None,
            form_body: None,
        },
    )
    .await;

    assert!(matches!(result, Err(AppError::BadRequest(_))));
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&app.pool)
        .await
        .expect("read user count");
    assert_eq!(users, 0, "the failed first sign-in rolls its user back");
}

#[sqlx::test]
async fn link_unique_violation_is_reported_as_already_linked(pool: sqlx::PgPool) {
    crate::db::migrate(&pool).await.expect("schema is migrated");
    let owner = crate::db::repo::users::create(&pool, Some("Account owner"), None)
        .await
        .expect("create account for provider link");
    install_identity_collision_trigger(&pool).await;

    let binding = BindingValue::generate();
    let state = "state-concurrent-link";
    crate::auth::attempt::create(
        &pool,
        &PendingAttempt {
            state: state.to_owned(),
            provider: ProviderKind::Google,
            nonce: "nonce".to_owned(),
            pkce_verifier: "verifier".to_owned(),
            link_user_id: Some(owner.id),
            redirect_to: None,
            expires_at: Utc::now() + ATTEMPT_TTL,
            binding_hash: binding.hash(),
        },
    )
    .await
    .expect("create link attempt");

    let config: crate::config::Config = serde_json::from_value(serde_json::json!({
        "database": {
            "url": "postgres://pinvoyager:pinvoyager@localhost/pinvoyager",
            "max_connections": 1,
            "min_connections": 0,
            "acquire_timeout_secs": 2
        },
        "auth": { "base_url": "http://localhost:8080/" }
    }))
    .expect("test config deserializes");
    let app = crate::state::AppState::new(
        pool,
        std::sync::Arc::new(config),
        crate::auth::ProviderRegistry::for_test(
            ProviderKind::Google,
            Box::new(StaticIdentityProvider),
        ),
        crate::photos::worker::VariantQueue::detached(),
    );
    let jar = CookieJar::new().add(
        Cookie::build((BINDING_COOKIE, binding.expose().to_owned()))
            .path("/")
            .build(),
    );

    let result = complete(
        &app,
        ProviderKind::Google,
        jar,
        Callback {
            code: Some("code"),
            state: Some(state),
            error: None,
            form_body: None,
        },
    )
    .await;

    assert!(matches!(result, Err(AppError::Conflict(message)) if message == ALREADY_LINKED));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM identities")
        .fetch_one(&app.pool)
        .await
        .expect("read identity count");
    assert_eq!(count, 0, "failed link does not attach an identity");
}

#[sqlx::test]
async fn callback_rejects_an_identity_labelled_as_another_provider(pool: sqlx::PgPool) {
    crate::db::migrate(&pool).await.expect("schema is migrated");
    let binding = BindingValue::generate();
    let state = "state-mislabelled-provider";
    crate::auth::attempt::create(
        &pool,
        &PendingAttempt {
            state: state.to_owned(),
            provider: ProviderKind::Google,
            nonce: "nonce".to_owned(),
            pkce_verifier: "verifier".to_owned(),
            link_user_id: None,
            redirect_to: None,
            expires_at: Utc::now() + ATTEMPT_TTL,
            binding_hash: binding.hash(),
        },
    )
    .await
    .expect("create pending attempt");

    let config: crate::config::Config = serde_json::from_value(serde_json::json!({
        "database": {
            "url": "postgres://pinvoyager:pinvoyager@localhost/pinvoyager",
            "max_connections": 1,
            "min_connections": 0,
            "acquire_timeout_secs": 2
        },
        "auth": { "base_url": "http://localhost:8080/" }
    }))
    .expect("test config deserializes");
    let app = crate::state::AppState::new(
        pool,
        std::sync::Arc::new(config),
        crate::auth::ProviderRegistry::for_test(
            ProviderKind::Google,
            Box::new(MislabelledProvider),
        ),
        crate::photos::worker::VariantQueue::detached(),
    );
    let jar = CookieJar::new().add(
        Cookie::build((BINDING_COOKIE, binding.expose().to_owned()))
            .path("/")
            .build(),
    );

    let result = complete(
        &app,
        ProviderKind::Google,
        jar,
        Callback {
            code: Some("code"),
            state: Some(state),
            error: None,
            form_body: None,
        },
    )
    .await;

    assert!(matches!(result, Err(AppError::BadRequest(_))));
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_attempts")
        .fetch_one(&app.pool)
        .await
        .expect("read consumed attempt count");
    assert_eq!(remaining, 0);
}

#[test]
fn session_cookie_attributes_match_session_configuration() {
    let config = SessionConfig {
        idle_timeout_secs: 86_400,
        absolute_timeout_secs: 604_800,
        cookie_name: "pv_session".to_owned(),
        cookie_secure: false,
    };
    let token = SessionToken::generate();

    let cookie = session_cookie(&config, &token);

    assert_eq!(cookie.name(), "pv_session");
    assert_eq!(cookie.value(), token.expose());
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.secure(), Some(false));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(
        cookie.max_age().map(|age| age.whole_seconds()),
        Some(86_400)
    );
}

#[test]
fn binding_cookie_is_secure_and_expires_with_the_attempt() {
    let cookie = binding_cookie(&BindingValue::generate());
    let removal = binding_cookie_removal();

    assert_eq!(cookie.name(), BINDING_COOKIE);
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::None));
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(
        cookie.max_age().map(|age| age.whole_seconds()),
        Some(ATTEMPT_TTL.num_seconds())
    );
    assert_eq!(removal.name(), BINDING_COOKIE);
    assert_eq!(removal.value(), "");
    assert_eq!(removal.max_age().map(|age| age.whole_seconds()), Some(0));
}

#[test]
fn cookie_seconds_clamp_instead_of_wrapping() {
    assert_eq!(clamp_secs(0), 0);
    assert_eq!(clamp_secs(i64::MAX as u64), i64::MAX);
    assert_eq!(clamp_secs(u64::MAX), i64::MAX);
}

#[test]
fn provider_parser_accepts_known_names_and_hides_unknown_names() {
    assert_eq!(parse_provider("google").unwrap(), ProviderKind::Google);
    assert_eq!(parse_provider("apple").unwrap(), ProviderKind::Apple);
    assert!(matches!(
        parse_provider("not-configured"),
        Err(AppError::NotFound)
    ));
}

#[test]
fn authentication_failures_are_generic_except_missing_providers() {
    assert!(matches!(
        auth_failed(AuthError::ProviderNotConfigured),
        AppError::NotFound
    ));
    for error in [
        AuthError::UnknownState,
        AuthError::AttemptExpired,
        AuthError::TokenVerification("secret detail".to_owned()),
        AuthError::Provider("provider detail".to_owned()),
        AuthError::IdentityAlreadyLinked,
        AuthError::Configuration("configuration detail".to_owned()),
    ] {
        match auth_failed(error) {
            AppError::BadRequest(message) => assert_eq!(message, GENERIC_FAILURE),
            other => panic!("unexpected auth failure response: {other:?}"),
        }
    }
}

#[test]
fn redirect_builds_the_requested_status_and_rejects_invalid_headers() {
    let response =
        redirect(StatusCode::FOUND, "https://provider.example/authorize").expect("valid redirect");
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://provider.example/authorize"
    );

    assert!(matches!(
        redirect(StatusCode::FOUND, "/ok\r\nSet-Cookie: forged"),
        Err(AppError::Internal(_))
    ));
}

#[test]
fn form_field_lookup_returns_the_first_match_or_none() {
    let fields = vec![
        ("state".to_owned(), "first".to_owned()),
        ("state".to_owned(), "second".to_owned()),
    ];
    assert_eq!(field(&fields, "state"), Some("first"));
    assert_eq!(field(&fields, "code"), None);
}

#[test]
fn callback_success_returns_the_response_and_new_cookie() {
    let response = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/")
        .body(axum::body::Body::empty())
        .expect("valid response");
    let jar = CookieJar::new().add(Cookie::build(("session", "token")).path("/").build());

    let response = to_browser(
        Ok((jar, response)),
        CookieJar::new(),
        ProviderKind::Google,
        &HeaderMap::new(),
    );

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    assert!(response.headers().contains_key(header::SET_COOKIE));
}

#[test]
fn callback_not_found_stays_a_not_found_response() {
    let response = to_browser(
        Err(AppError::NotFound),
        CookieJar::new(),
        ProviderKind::Google,
        &HeaderMap::new(),
    );

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!response.headers().contains_key(header::LOCATION));
}

#[test]
fn callback_cancellation_redirects_to_the_canceled_screen() {
    let response = to_browser(
        Err(AppError::BadRequest(CANCELED.to_owned())),
        CookieJar::new(),
        ProviderKind::Apple,
        &HeaderMap::new(),
    );

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/auth/callback?status=canceled&provider=apple"
    );
}

#[test]
fn callback_conflict_redirects_to_the_conflict_screen() {
    let response = to_browser(
        Err(AppError::Conflict("already linked".to_owned())),
        CookieJar::new(),
        ProviderKind::Google,
        &HeaderMap::new(),
    );

    assert_eq!(
        response.headers()[header::LOCATION],
        "/auth/callback?status=conflict&provider=google"
    );
}

#[test]
fn callback_failure_includes_only_safe_request_reference_characters() {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("abc<script>-123"));

    let response = to_browser(
        Err(AppError::BadRequest("internal detail".to_owned())),
        CookieJar::new(),
        ProviderKind::Facebook,
        &headers,
    );

    assert_eq!(
        response.headers()[header::LOCATION],
        "/auth/callback?status=failed&provider=facebook&ref=abcscript-123"
    );
}

#[test]
fn callback_failure_omits_an_empty_or_invalid_request_reference() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-request-id",
        HeaderValue::from_bytes(b"\xff").expect("obs-text header value is valid"),
    );

    let response = to_browser(
        Err(AppError::Unauthorized),
        CookieJar::new(),
        ProviderKind::Microsoft,
        &headers,
    );

    assert_eq!(
        response.headers()[header::LOCATION],
        "/auth/callback?status=failed&provider=microsoft"
    );
}

/// The values a phishing attempt actually uses. Each of these is a way of
/// naming another origin with a string that starts with a slash, or of
/// smuggling a second header past `Location`.
#[test]
fn off_site_destinations_are_refused() {
    for candidate in [
        "https://evil.example/",
        "//evil.example/",
        "/\\evil.example",
        "/\\/evil.example",
        "http:/evil.example",
        "evil.example",
        "/discover\r\nSet-Cookie: a=b",
        "/discover\nX: y",
        // Not on the allowlist, and on-site is not by itself enough: an
        // unknown path is a typo or a probe.
        "/admin",
        // Prefix without a separator — the reason the check looks at the
        // byte after the match.
        "/settings-evil.example",
        "/voyagesomething",
    ] {
        assert_eq!(safe_redirect(candidate), None, "accepted {candidate:?}");
    }
}

#[test]
fn on_site_destinations_are_kept_verbatim() {
    for candidate in [
        "/",
        "/discover",
        "/settings",
        "/voyages/0192f0c2-1a2b-7c3d-8e4f-5a6b7c8d9e0f",
        "/discover?near=me#top",
    ] {
        assert_eq!(
            safe_redirect(candidate).as_deref(),
            Some(candidate),
            "refused {candidate:?}"
        );
    }
}

#[test]
fn an_over_long_destination_is_refused() {
    let long = format!("/discover?q={}", "a".repeat(MAX_REDIRECT_LEN));
    assert_eq!(safe_redirect(&long), None);
}

#[sqlx::test]
async fn unique_violation_recognizes_postgres_duplicate_key_errors(pool: sqlx::PgPool) {
    sqlx::query("CREATE TABLE auth_unique_probe (value text UNIQUE)")
        .execute(&pool)
        .await
        .expect("create temporary uniqueness probe");
    sqlx::query("INSERT INTO auth_unique_probe (value) VALUES ('same')")
        .execute(&pool)
        .await
        .expect("insert first value");
    let duplicate = sqlx::query("INSERT INTO auth_unique_probe (value) VALUES ('same')")
        .execute(&pool)
        .await
        .expect_err("duplicate value violates the unique constraint");

    assert!(unique_violation(&duplicate));
    assert!(!unique_violation(&sqlx::Error::RowNotFound));
}

#[test]
fn identity_insert_mappers_preserve_non_unique_database_errors() {
    assert!(matches!(
        first_sign_in_identity_error(sqlx::Error::RowNotFound),
        AppError::Database(sqlx::Error::RowNotFound)
    ));
    assert!(matches!(
        link_identity_error(sqlx::Error::RowNotFound),
        AppError::Database(sqlx::Error::RowNotFound)
    ));
}

/// A stored value is validated again on use, so an unvalidated row cannot
/// become a redirect.
#[test]
fn a_stored_destination_falls_back_when_it_no_longer_passes() {
    assert_eq!(
        destination(Some("https://evil.example/"), SIGNED_IN_DESTINATION),
        SIGNED_IN_DESTINATION
    );
    assert_eq!(destination(None, LINKED_DESTINATION), LINKED_DESTINATION);
    assert_eq!(
        destination(Some("/settings"), SIGNED_IN_DESTINATION),
        "/settings"
    );
}

/// Apple's body is form-encoded, and its `user` payload is JSON inside one
/// of the fields — so the field parser has to survive `{`, `"` and `%`.
#[test]
fn form_fields_are_read_from_an_apple_style_body() {
    let body = "state=abc&code=xyz&user=%7B%22name%22%3A%7B%22firstName%22%3A%22Ada%22%7D%7D";
    let fields: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();

    assert_eq!(field(&fields, "state"), Some("abc"));
    assert_eq!(field(&fields, "code"), Some("xyz"));
    assert_eq!(field(&fields, "error"), None);
    assert_eq!(
        field(&fields, "user"),
        Some(r#"{"name":{"firstName":"Ada"}}"#)
    );
}
