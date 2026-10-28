use super::*;

use crate::test_support as support;

#[test]
fn a_configured_issuer_keeps_the_spelling_the_provider_uses() {
    // The `Url` type has already appended a slash to the first of these by
    // the time configuration is parsed. Both must come out identical, and
    // identical to what Google's discovery document says.
    for configured in [
        "https://accounts.google.com",
        "https://accounts.google.com/",
    ] {
        let url = Url::parse(configured).expect("test URL should parse");
        let issuer = issuer_url(&url).expect("issuer should be accepted");
        assert_eq!(issuer.as_str(), "https://accounts.google.com");
    }
}

#[test]
fn an_issuer_with_a_path_keeps_it() {
    // Microsoft's issuer is a path under a shared host, and the tenant
    // segment is the part that must survive.
    let url = Url::parse("https://login.microsoftonline.com/tenant-id/v2.0/")
        .expect("test URL should parse");
    let issuer = issuer_url(&url).expect("issuer should be accepted");
    assert_eq!(
        issuer.as_str(),
        "https://login.microsoftonline.com/tenant-id/v2.0"
    );
}

#[test]
fn a_non_http_issuer_is_a_configuration_error() {
    let url = Url::parse("file:///tmp/issuer").expect("file URL is valid URL syntax");
    let error = issuer_url(&url).expect_err("OIDC issuers must use HTTP or HTTPS");

    assert!(
        matches!(error, AuthError::Configuration(message) if message.contains("invalid issuer URL"))
    );
}

async fn core_for_test(
    mock: &support::MockOidcProvider,
    extra_authorize_params: Vec<(&'static str, &'static str)>,
) -> OidcCore {
    OidcCore::discover(
        ProviderKind::Google,
        mock.issuer(),
        support::CLIENT_ID,
        Arc::new(StaticClientSecret::new(&SecretString::from(
            support::CLIENT_SECRET,
        ))),
        RedirectUrl::new("http://pinvoyager.example/auth/google/callback".to_owned())
            .expect("valid callback URL"),
        vec![Scope::new("profile".to_owned())],
        extra_authorize_params,
    )
    .await
    .expect("mock discovery and JWKS should succeed")
}

#[tokio::test]
async fn authorize_includes_configured_scopes_and_extra_parameters() {
    let mock = support::MockOidcProvider::start().await;
    let core = core_for_test(&mock, vec![("prompt", "consent")]).await;

    let redirect = core.authorize().expect("build authorization redirect");
    let query: std::collections::HashMap<_, _> = redirect.url.query_pairs().into_owned().collect();

    assert_eq!(query.get("prompt").map(String::as_str), Some("consent"));
    let scope = query.get("scope").expect("authorization includes scope");
    assert!(scope.split_whitespace().any(|value| value == "openid"));
    assert!(scope.split_whitespace().any(|value| value == "profile"));
    assert!(!redirect.state.is_empty());
    assert!(!redirect.nonce.is_empty());
    assert!(!redirect.pkce_verifier.is_empty());
}

#[tokio::test]
async fn jwks_refresh_fetches_and_replaces_cached_metadata_after_the_refetch_floor() {
    let mock = support::MockOidcProvider::start().await;
    let core = core_for_test(&mock, Vec::new()).await;
    let requests_before_refresh = mock.jwks_requests();
    assert!(
        requests_before_refresh >= 1,
        "discovery fetches the initial JWKS"
    );
    core.metadata
        .write()
        .expect("metadata lock is healthy")
        .jwks_fetched_at = Instant::now() - JWKS_MIN_REFETCH_INTERVAL - Duration::from_secs(1);

    core.refresh_jwks()
        .await
        .expect("a reachable provider can refresh its JWKS");

    assert_eq!(mock.jwks_requests(), requests_before_refresh + 1);
}

#[tokio::test]
async fn standard_provider_exposes_its_configured_kind() {
    use crate::auth::AuthProvider as _;

    let mock = support::MockOidcProvider::start().await;
    let provider = StandardOidcProvider {
        core: core_for_test(&mock, Vec::new()).await,
    };

    assert_eq!(provider.kind(), ProviderKind::Google);
}

#[tokio::test]
async fn standard_provider_authorizes_and_exchanges_without_an_apple_form_body() {
    use crate::auth::AuthProvider as _;

    let mock = support::MockOidcProvider::start().await;
    let provider = StandardOidcProvider {
        core: core_for_test(&mock, Vec::new()).await,
    };
    let redirect = provider.authorize().expect("authorization should succeed");
    assert_eq!(redirect.url.host_str(), mock.issuer().host_str());

    let code = mock
        .token()
        .claim(
            "picture",
            serde_json::json!("https://images.example/avatar.jpg"),
        )
        .issue();
    let identity = provider
        .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
        .await
        .expect("the adapter should delegate the verified OIDC exchange");
    assert_eq!(identity.provider, ProviderKind::Google);
    assert_eq!(identity.subject, support::SUBJECT);
    assert_eq!(
        identity.avatar_url.as_deref(),
        Some("https://images.example/avatar.jpg")
    );
}

#[tokio::test]
async fn exchange_reports_a_missing_token_endpoint_as_configuration_error() {
    let mock = support::MockOidcProvider::start().await;
    mock.omit_token_endpoint();
    let core = core_for_test(&mock, Vec::new()).await;

    let error = core
        .exchange("unused-code", support::PKCE_VERIFIER, support::NONCE)
        .await
        .expect_err("discovery omitted the required code-exchange endpoint");

    assert!(
        matches!(error, AuthError::Configuration(message) if message.contains("no token endpoint"))
    );
}

#[tokio::test]
async fn exchange_rejects_a_token_response_without_an_id_token() {
    let mock = support::MockOidcProvider::start().await;
    mock.omit_id_token();
    let core = core_for_test(&mock, Vec::new()).await;
    let code = mock.token().issue();

    let error = core
        .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
        .await
        .expect_err("an access token without an ID token cannot authenticate a user");

    assert!(
        matches!(error, AuthError::TokenVerification(message) if message.contains("no ID token"))
    );
}

#[tokio::test]
async fn unknown_signing_key_refetches_jwks_once_then_still_rejects_the_token() {
    let mock = support::MockOidcProvider::start().await;
    let core = core_for_test(&mock, Vec::new()).await;
    core.metadata
        .write()
        .expect("metadata lock is healthy")
        .jwks_fetched_at = Instant::now() - JWKS_MIN_REFETCH_INTERVAL - Duration::from_secs(1);
    let before = mock.jwks_requests();
    let code = mock.token().signed_by_an_unknown_key().issue();

    let error = core
        .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
        .await
        .expect_err("a key absent from refreshed JWKS must still be rejected");

    assert!(matches!(error, AuthError::TokenVerification(_)));
    assert_eq!(mock.jwks_requests(), before + 1);
}

#[tokio::test]
async fn stopped_provider_is_reported_at_discovery_exchange_and_key_refresh() {
    let mock = support::MockOidcProvider::start().await;
    let core = core_for_test(&mock, Vec::new()).await;
    let issuer = mock.issuer().clone();
    drop(mock);
    tokio::task::yield_now().await;
    assert!(matches!(
        core.exchange("unused", support::PKCE_VERIFIER, support::NONCE)
            .await,
        Err(AuthError::Provider(_))
    ));
    core.metadata.write().expect("metadata").jwks_fetched_at =
        Instant::now() - JWKS_MIN_REFETCH_INTERVAL - Duration::from_secs(1);
    assert!(matches!(
        core.refresh_jwks().await,
        Err(AuthError::Provider(_))
    ));
    let discovered = OidcCore::discover(
        ProviderKind::Google,
        &issuer,
        support::CLIENT_ID,
        Arc::new(StaticClientSecret::new(&SecretString::from(
            support::CLIENT_SECRET,
        ))),
        RedirectUrl::new("http://pinvoyager.example/auth/google/callback".to_owned()).expect("URL"),
        vec![],
        vec![],
    )
    .await;
    assert!(matches!(discovered, Err(AuthError::Configuration(_))));
}

const PERSONAL_TENANT: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";
const BUSINESS_TENANT: &str = "11111111-2222-3333-4444-555555555555";

#[test]
fn only_microsoft_common_uses_tenant_independent_discovery() {
    for (kind, url, expected) in [
        (
            ProviderKind::Microsoft,
            "https://login.microsoftonline.com/common/v2.0",
            true,
        ),
        (
            ProviderKind::Microsoft,
            "https://login.microsoftonline.com/common/v2.0/",
            true,
        ),
        (
            ProviderKind::Google,
            "https://login.microsoftonline.com/common/v2.0",
            false,
        ),
        (
            ProviderKind::Microsoft,
            "https://evil.example/common/v2.0",
            false,
        ),
        (
            ProviderKind::Microsoft,
            "http://login.microsoftonline.com/common/v2.0",
            false,
        ),
        (
            ProviderKind::Microsoft,
            "https://login.microsoftonline.com/tenant/v2.0",
            false,
        ),
    ] {
        assert_eq!(
            microsoft_common(kind, &Url::parse(url).expect("valid authority URL")),
            expected
        );
    }
}

async fn microsoft_core(mock: &support::MockOidcProvider, key_issuer: Option<&str>) -> OidcCore {
    let mut core = core_for_test(mock, vec![]).await;
    core.kind = ProviderKind::Microsoft;
    mock.use_microsoft_common_metadata();
    let (metadata, mut keys) = discover_microsoft_metadata(
        &issuer_url(mock.issuer()).expect("valid mock issuer"),
        &core.http,
    )
    .await
    .expect("valid Microsoft discovery or token exchange");
    for key in keys["keys"].as_array_mut().expect("JWKS keys are an array") {
        key.as_object_mut()
            .expect("JWK is an object")
            .remove("issuer");
    }
    let mut cached = core.metadata.write().expect("metadata lock is healthy");
    cached.metadata = metadata;
    if let Some(issuer) = key_issuer {
        for key in keys["keys"].as_array_mut().expect("JWKS keys are an array") {
            key["issuer"] = serde_json::json!(issuer);
        }
    }
    cached.microsoft_jwks = Some(keys);
    drop(cached);
    core
}

#[tokio::test]
async fn microsoft_accepts_personal_and_business_accounts() {
    let mock = support::MockOidcProvider::start().await;
    let core = microsoft_core(&mock, Some(MICROSOFT_ISSUER_TEMPLATE)).await;
    for tenant in [PERSONAL_TENANT, BUSINESS_TENANT] {
        let issuer = MICROSOFT_ISSUER_TEMPLATE.replace("{tenantid}", tenant);
        let code = mock
            .token()
            .issuer(&issuer)
            .claim("tid", serde_json::json!(tenant))
            .issue();
        let identity = core
            .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
            .await
            .expect("valid Microsoft discovery or token exchange");
        assert_eq!(identity.provider, ProviderKind::Microsoft);
        assert_eq!(identity.subject, support::SUBJECT);
    }
}

#[tokio::test]
async fn microsoft_rejects_invalid_tenants_issuers_and_security_claims() {
    let mock = support::MockOidcProvider::start().await;
    let core = microsoft_core(&mock, Some(MICROSOFT_ISSUER_TEMPLATE)).await;
    let issuer = MICROSOFT_ISSUER_TEMPLATE.replace("{tenantid}", BUSINESS_TENANT);
    for (tid, iss) in [
        (serde_json::Value::Null, issuer.clone()),
        (serde_json::json!("not-a-uuid"), issuer.clone()),
        (serde_json::json!("../common"), issuer.clone()),
        (
            serde_json::json!(BUSINESS_TENANT),
            "https://evil.example/v2.0".into(),
        ),
        (serde_json::json!(PERSONAL_TENANT), issuer.clone()),
    ] {
        let code = mock.token().issuer(&iss).claim("tid", tid).issue();
        assert!(matches!(
            core.exchange(&code, support::PKCE_VERIFIER, support::NONCE)
                .await,
            Err(AuthError::TokenVerification(_))
        ));
    }
    for (claim, value) in [
        ("aud", serde_json::json!("wrong-client")),
        ("nonce", serde_json::json!("wrong-nonce")),
        ("exp", serde_json::json!(1)),
    ] {
        let code = mock
            .token()
            .issuer(&issuer)
            .claim("tid", serde_json::json!(BUSINESS_TENANT))
            .claim(claim, value)
            .issue();
        assert!(
            core.exchange(&code, support::PKCE_VERIFIER, support::NONCE)
                .await
                .is_err()
        );
    }
    let code = mock
        .token()
        .issuer(&issuer)
        .claim("tid", serde_json::json!(BUSINESS_TENANT))
        .signed_by_an_unknown_key()
        .issue();
    assert!(
        core.exchange(&code, support::PKCE_VERIFIER, support::NONCE)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn microsoft_requires_a_signing_key_scoped_to_the_token_issuer() {
    let issuer = MICROSOFT_ISSUER_TEMPLATE.replace("{tenantid}", PERSONAL_TENANT);
    for scope in [
        None,
        Some("https://evil.example/v2.0"),
        Some(&issuer),
        Some(MICROSOFT_ISSUER_TEMPLATE),
    ] {
        let mock = support::MockOidcProvider::start().await;
        let core = microsoft_core(&mock, scope).await;
        let code = mock
            .token()
            .issuer(&issuer)
            .claim("tid", serde_json::json!(PERSONAL_TENANT))
            .issue();
        let result = core
            .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
            .await;
        assert_eq!(
            result.is_ok(),
            scope == Some(&issuer) || scope == Some(MICROSOFT_ISSUER_TEMPLATE)
        );
    }
}

#[tokio::test]
async fn microsoft_discovery_rejects_an_unexpected_issuer() {
    let mock = support::MockOidcProvider::start().await;
    assert!(matches!(
        discover_microsoft_metadata(
            &issuer_url(mock.issuer()).expect("valid mock issuer"),
            &provider_http_client().expect("build provider HTTP client")
        )
        .await,
        Err(AuthError::Configuration(_))
    ));
}

#[tokio::test]
async fn microsoft_key_refresh_preserves_key_issuer_restrictions() {
    let mock = support::MockOidcProvider::start().await;
    let core = microsoft_core(&mock, Some("https://evil.example")).await;
    core.metadata
        .write()
        .expect("metadata lock is healthy")
        .jwks_fetched_at = Instant::now() - JWKS_MIN_REFETCH_INTERVAL - Duration::from_secs(1);
    let issuer = MICROSOFT_ISSUER_TEMPLATE.replace("{tenantid}", BUSINESS_TENANT);
    let code = mock
        .token()
        .issuer(&issuer)
        .claim("tid", serde_json::json!(BUSINESS_TENANT))
        .issue();
    let before = mock.jwks_requests();
    core.exchange(&code, support::PKCE_VERIFIER, support::NONCE)
        .await
        .expect("valid Microsoft discovery or token exchange");
    assert_eq!(mock.jwks_requests(), before + 1);
    let keys = core
        .metadata
        .read()
        .expect("metadata lock is healthy")
        .microsoft_jwks
        .clone()
        .expect("Microsoft signing keys are cached");
    assert_eq!(keys["keys"][0]["issuer"], MICROSOFT_ISSUER_TEMPLATE);
}

#[test]
fn microsoft_tenant_parsing_rejects_malformed_tokens_and_noncanonical_ids() {
    for token in ["", "header.!.signature", "header.bm90LWpzb24.signature"] {
        assert!(microsoft_token_issuer(token).is_err());
    }
    for tenant in [
        serde_json::Value::Null,
        serde_json::json!(PERSONAL_TENANT.to_uppercase()),
        serde_json::json!(PERSONAL_TENANT.replace('-', "")),
    ] {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({"tid": tenant}))
                .expect("serialize tenant claim"),
        );
        assert!(microsoft_token_issuer(&format!("header.{payload}.signature")).is_err());
    }
}

#[tokio::test]
async fn microsoft_metadata_fetch_reports_transport_status_and_json_failures() {
    use axum::{Router, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("mock server address");
    let app = Router::new().route("/invalid", get(|| async { "not-json" }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    let http = provider_http_client().expect("provider HTTP client");
    for path in ["missing", "invalid"] {
        assert!(matches!(
            fetch_json(&http, &format!("http://{addr}/{path}")).await,
            Err(AuthError::Provider(_))
        ));
    }
    server.abort();
    let _ = server.await;
    assert!(matches!(
        fetch_json(&http, &format!("http://{addr}/invalid")).await,
        Err(AuthError::Provider(_))
    ));
}

#[tokio::test]
async fn google_does_not_request_email() {
    use crate::auth::AuthProvider as _;
    let mock = support::MockOidcProvider::start().await;
    let config = crate::config::OidcProviderConfig {
        client_id: support::CLIENT_ID.into(),
        client_secret: SecretString::from(support::CLIENT_SECRET),
        issuer: mock.issuer().clone(),
    };
    let google = StandardOidcProvider::discover(
        ProviderKind::Google,
        &config,
        &Url::parse("https://pinvoyager.example/").expect("URL"),
    )
    .await
    .expect("provider");
    let authorization = google.authorize().expect("authorization");
    let scope = authorization
        .url
        .query_pairs()
        .find(|(key, _)| key == "scope")
        .expect("scope")
        .1
        .into_owned();
    assert!(!scope.split_whitespace().any(|scope| scope == "email"));
}

#[tokio::test]
async fn microsoft_discovery_rejects_malformed_documents_and_unavailable_signing_keys() {
    use axum::{Json, Router, http::StatusCode, response::IntoResponse as _, routing::get};
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener");
    let address = listener.local_addr().expect("mock address");
    let mode = Arc::new(AtomicU8::new(0));
    let metadata_mode = mode.clone();
    let keys_mode = mode.clone();
    let app = Router::new().route("/.well-known/openid-configuration",get(move || {
        let mode = metadata_mode.load(Ordering::Relaxed);
        async move {
            if mode==3 { return StatusCode::SERVICE_UNAVAILABLE.into_response(); }
            if mode==0 { return Json(serde_json::json!({"issuer":MICROSOFT_ISSUER_TEMPLATE})).into_response(); }
            Json(serde_json::json!({"issuer":MICROSOFT_ISSUER_TEMPLATE,"authorization_endpoint":format!("http://{address}/authorize"),"token_endpoint":format!("http://{address}/token"),"jwks_uri":format!("http://{address}/jwks"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"]})).into_response()
        }
    })).route("/jwks",get(move || {
        let mode = keys_mode.load(Ordering::Relaxed);
        async move {
            if mode==2 { StatusCode::SERVICE_UNAVAILABLE.into_response() }
            else { Json(serde_json::json!({"keys":"not a key array"})).into_response() }
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock server");
    });
    let issuer = IssuerUrl::new(format!("http://{address}")).expect("issuer");
    let http = provider_http_client().expect("HTTP client");
    for value in 0..4 {
        mode.store(value, Ordering::Relaxed);
        let result = discover_microsoft_metadata(&issuer, &http).await;
        if value < 2 {
            assert!(matches!(result, Err(AuthError::Configuration(_))));
        } else {
            assert!(matches!(result, Err(AuthError::Provider(_))));
        }
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn apple_deletion_requires_confirmation_from_the_revocation_endpoint() {
    let mock = support::MockOidcProvider::start().await;
    let mut core = OidcCore::discover(
        ProviderKind::Apple,
        mock.issuer(),
        support::CLIENT_ID,
        Arc::new(StaticClientSecret::new(&SecretString::from(
            support::CLIENT_SECRET,
        ))),
        RedirectUrl::new("http://pinvoyager.example/auth/apple/callback".into()).expect("callback"),
        vec![],
        vec![],
    )
    .await
    .expect("provider");
    core.revocation_url =
        Url::parse("http://127.0.0.1:1/unreachable").expect("revocation endpoint");
    let code = mock.token().issue();
    let result = core
        .exchange_for_deletion(
            &code,
            support::PKCE_VERIFIER,
            support::NONCE,
            support::SUBJECT,
        )
        .await;
    assert!(
        matches!(result,Err(AuthError::Provider(message)) if message=="Apple revocation could not be reached")
    );
    assert!(mock.revocations().is_empty());
}
