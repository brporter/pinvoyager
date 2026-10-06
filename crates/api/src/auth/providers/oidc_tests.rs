use super::*;

#[path = "../../../tests/support/mod.rs"]
mod support;

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

    let code = mock.token().issue();
    let identity = provider
        .exchange(&code, support::PKCE_VERIFIER, support::NONCE, None)
        .await
        .expect("the adapter should delegate the verified OIDC exchange");
    assert_eq!(identity.provider, ProviderKind::Google);
    assert_eq!(identity.subject, support::SUBJECT);
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
