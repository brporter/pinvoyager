use super::*;
use crate::test_support as support;
use secrecy::SecretString;

/// A throwaway P-256 key pair generated for this test file and used nowhere
/// else — the same shape as the `.p8` Apple issues:
///
/// ```sh
/// openssl ecparam -genkey -noout -name prime256v1 \
///     | openssl pkcs8 -topk8 -nocrypt -out ec-private.pem
/// openssl ec -in ec-private.pem -pubout
/// ```
///
/// Having the public half means these tests verify a real ES256 signature
/// rather than trusting that `encode` did something.
const TEST_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg5XYNJMqUWV4t8Ive\n\
        aMzdW9Hywi9Gm8seX4KiD21HUhWhRANCAATtgsS2FtzL4WAKAi3QEq+IOOigdivH\n\
        EwyiPEKo7DuSlwxiTqRmG18C/R+nl1hA1RBdGaijKfRjoQO9hyxWj7kq\n\
        -----END PRIVATE KEY-----\n";

const TEST_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
        MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE7YLEthbcy+FgCgIt0BKviDjooHYr\n\
        xxMMojxCqOw7kpcMYk6kZhtfAv0fp5dYQNUQXRmooyn0Y6EDvYcsVo+5Kg==\n\
        -----END PUBLIC KEY-----\n";

fn test_secret() -> AppleClientSecret {
    AppleClientSecret::new(&AppleProviderConfig {
        client_id: "com.example.pinvoyager.service".to_owned(),
        team_id: "ABCDE12345".to_owned(),
        key_id: "KEYID67890".to_owned(),
        private_key_pem: SecretString::from(TEST_KEY_PEM),
    })
    .expect("the test key should parse as PKCS#8 EC")
}

#[test]
fn a_signing_backend_panic_is_reported_as_configuration_error() {
    let result = catch_secret_probe(|| -> Result<openidconnect::ClientSecret, AuthError> {
        panic!("synthetic missing signing backend")
    });

    assert!(
        matches!(result, Err(AuthError::Configuration(message)) if message.contains("no crypto backend"))
    );
}

#[test]
fn an_unusable_signing_key_returns_a_configuration_error() {
    let mut secret = test_secret();
    secret.key = EncodingKey::from_secret(b"not an EC key");
    let now = Utc::now();

    let error = secret
        .mint(now, now + CLIENT_SECRET_LIFETIME)
        .expect_err("ES256 signing rejects an HMAC key");

    assert!(
        matches!(error, AuthError::Configuration(message) if message.contains("could not sign the Apple client secret"))
    );
}

/// Verifies the token the way Apple would: real signature check against the
/// public half of the signing key, then `aud`, `iss` and `exp`.
fn decode(token: &str) -> jsonwebtoken::TokenData<ClientSecretClaims> {
    let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
    validation.set_audience(&[CLIENT_SECRET_AUDIENCE]);
    validation.set_issuer(&["ABCDE12345"]);
    validation.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);

    let key = jsonwebtoken::DecodingKey::from_ec_pem(TEST_PUBLIC_KEY_PEM.as_bytes())
        .expect("the test public key should parse");

    jsonwebtoken::decode::<ClientSecretClaims>(token, &key, &validation)
        .expect("the minted secret should verify and validate")
}

#[test]
fn client_secret_carries_the_claims_apple_requires() {
    let now = Utc::now();
    let secret = test_secret();
    let token = secret.secret_at(now).expect("minting should succeed");
    let decoded = decode(&token);

    // The key ID is how Apple finds the public key. Getting it wrong looks
    // exactly like a bad signature from the outside.
    assert_eq!(decoded.header.kid.as_deref(), Some("KEYID67890"));
    assert_eq!(decoded.header.alg, Algorithm::ES256);

    // iss is the *team*, sub is the *Services ID*. Swapping them is the
    // classic Apple misconfiguration and both are opaque strings, so
    // nothing but a test catches it.
    assert_eq!(decoded.claims.iss, "ABCDE12345");
    assert_eq!(decoded.claims.sub, "com.example.pinvoyager.service");
    assert_eq!(decoded.claims.aud, CLIENT_SECRET_AUDIENCE);
    assert_eq!(decoded.claims.iat, now.timestamp());
}

#[test]
fn client_secret_lifetime_is_well_inside_apples_ceiling() {
    let now = Utc::now();
    let token = test_secret()
        .secret_at(now)
        .expect("minting should succeed");
    let decoded = decode(&token);

    let lifetime = decoded.claims.exp - decoded.claims.iat;
    assert!(lifetime > 0, "the secret must not be born expired");
    // Six months, the value Apple rejects beyond.
    assert!(
        lifetime <= 15_777_000,
        "lifetime {lifetime}s exceeds Apple's six-month maximum"
    );
}

#[test]
fn client_secret_is_cached_and_renewed_before_expiry() {
    let secret = test_secret();
    let now = Utc::now();

    let first = secret.secret_at(now).expect("minting should succeed");
    let again = secret
        .secret_at(now + Duration::seconds(1))
        .expect("minting should succeed");
    assert_eq!(
        first, again,
        "a live secret should be reused, not re-signed"
    );

    // Inside the renewal margin: still valid, but close enough that handing
    // it out risks it lapsing mid-request.
    let renewed = secret
        .secret_at(now + CLIENT_SECRET_LIFETIME - CLIENT_SECRET_RENEW_BEFORE)
        .expect("minting should succeed");
    assert_ne!(first, renewed, "a nearly-expired secret should be replaced");
}

#[test]
fn invalid_private_key_is_rejected_as_a_provider_configuration_error() {
    let result = AppleClientSecret::new(&AppleProviderConfig {
        client_id: "com.example.pinvoyager.service".to_owned(),
        team_id: "ABCDE12345".to_owned(),
        key_id: "KEYID67890".to_owned(),
        private_key_pem: SecretString::from("not a private key"),
    });

    let error = match result {
        Ok(_) => panic!("malformed private key must disable Apple at startup"),
        Err(error) => error,
    };
    assert!(matches!(error, AuthError::Configuration(_)));
    assert!(!error.to_string().contains("not a private key"));
}

#[test]
fn startup_probe_and_client_secret_source_mint_a_valid_token() {
    let secret = test_secret();

    secret
        .probe()
        .expect("valid signing key passes the startup probe");
    let client_secret = ClientSecretSource::client_secret(&secret)
        .expect("the configured client secret source mints a client secret");
    let decoded = decode(client_secret.secret());

    assert_eq!(decoded.claims.iss, "ABCDE12345");
}

#[tokio::test]
async fn apple_discovery_rejects_a_bad_key_before_network_discovery() {
    let config = AppleProviderConfig {
        client_id: "com.example.pinvoyager.service".to_owned(),
        team_id: "ABCDE12345".to_owned(),
        key_id: "KEYID67890".to_owned(),
        private_key_pem: SecretString::from("not a private key"),
    };
    let base_url = Url::parse("https://pinvoyager.example/").expect("valid test fixture");

    let error = match AppleProvider::discover(&config, &base_url, APPLE_ISSUER, false).await {
        Ok(_) => panic!("invalid signing key must fail before contacting Apple"),
        Err(error) => error,
    };

    assert!(matches!(error, AuthError::Configuration(_)));
}

#[tokio::test]
async fn apple_discovery_probes_its_key_and_uses_the_fixed_form_post_parameters() {
    use crate::auth::AuthProvider as _;

    let mock = support::MockOidcProvider::start().await;
    let config = AppleProviderConfig {
        client_id: "com.example.pinvoyager.service".to_owned(),
        team_id: "ABCDE12345".to_owned(),
        key_id: "KEYID67890".to_owned(),
        private_key_pem: SecretString::from(TEST_KEY_PEM),
    };
    let base_url = Url::parse("https://pinvoyager.example/").expect("valid test fixture");
    let apple = AppleProvider::discover(&config, &base_url, mock.issuer().as_str(), false)
        .await
        .expect("Apple can discover against the local OIDC provider");

    assert_eq!(apple.kind(), ProviderKind::Apple);
    let redirect = apple.authorize().expect("build Apple authorization");
    let query: std::collections::HashMap<_, _> = redirect.url.query_pairs().into_owned().collect();
    assert_eq!(
        query.get("response_mode").map(String::as_str),
        Some("form_post")
    );
    assert!(query.get("scope").is_some_and(|scope| scope == "openid"));
}

async fn apple_provider_against_mock(mock: &support::MockOidcProvider) -> AppleProvider {
    let core = OidcCore::discover(
        ProviderKind::Apple,
        mock.issuer(),
        "com.example.pinvoyager.service",
        Arc::new(test_secret()),
        openidconnect::RedirectUrl::new("http://pinvoyager.example/auth/apple/callback".to_owned())
            .expect("valid callback URL"),
        vec![],
        vec![("response_mode", "form_post")],
    )
    .await
    .expect("mock OIDC discovery should succeed");

    AppleProvider { core }
}

#[tokio::test]
async fn apple_provider_uses_form_post_without_requesting_profile_names() {
    use crate::auth::AuthProvider as _;

    let mock = support::MockOidcProvider::start().await;
    let apple = apple_provider_against_mock(&mock).await;

    assert_eq!(apple.kind(), ProviderKind::Apple);
    let redirect = apple.authorize().expect("build Apple authorization");
    let query: std::collections::HashMap<_, _> = redirect.url.query_pairs().into_owned().collect();
    assert_eq!(
        query.get("response_mode").map(String::as_str),
        Some("form_post")
    );
    assert!(query.get("scope").is_some_and(|scope| scope == "openid"));

    let code = mock
        .token()
        .audience("com.example.pinvoyager.service")
        .issue();
    let identity = apple
        .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
        .await
        .expect("valid mock ID token verifies");

    assert_eq!(identity.provider, ProviderKind::Apple);
}

#[tokio::test]
async fn apple_bootstrap_requests_email_and_requires_a_verified_token_claim() {
    use crate::auth::AuthProvider as _;
    let mock = support::MockOidcProvider::start().await;
    let config = AppleProviderConfig {
        client_id: support::CLIENT_ID.into(),
        team_id: "ABCDE12345".into(),
        key_id: "KEYID67890".into(),
        private_key_pem: SecretString::from(TEST_KEY_PEM),
    };
    let apple = AppleProvider::discover(
        &config,
        &Url::parse("https://pinvoyager.example/").expect("URL"),
        mock.issuer().as_str(),
        true,
    )
    .await
    .expect("provider");
    let authorization = apple.authorize().expect("authorization");
    let scope = authorization
        .url
        .query_pairs()
        .find(|(key, _)| key == "scope")
        .expect("scope")
        .1
        .into_owned();
    assert!(scope.split_whitespace().any(|scope| scope == "email"));
    for (verified, expected) in [
        (serde_json::json!(true), true),
        (serde_json::json!("true"), true),
        (serde_json::json!(false), false),
        (serde_json::json!("false"), false),
        (serde_json::Value::Null, false),
    ] {
        let code = mock
            .token()
            .claim("email", serde_json::json!("relay@privaterelay.appleid.com"))
            .claim("email_verified", verified)
            .issue();
        let identity = apple
            .exchange(&code, support::PKCE_VERIFIER, support::NONCE)
            .await
            .expect("identity");
        assert_eq!(
            identity.admin_email.as_deref(),
            expected.then_some("relay@privaterelay.appleid.com")
        );
    }
}
