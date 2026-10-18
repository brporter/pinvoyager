//! OIDC verification against a loopback provider with crafted tokens.
//!
//! Each rejection changes one token property while preserving valid defaults.
//! The tests exercise discovery, code exchange, and signature/claim verification
//! without a database or external provider credentials.

mod support;

use pinvoyager_api::auth::provider::VerifiedIdentity;
use pinvoyager_api::auth::{AuthError, AuthProvider, ProviderKind, ProviderRegistry};
use pinvoyager_api::config::AuthConfig;
use serde_json::json;
use support::{CLIENT_ID, CLIENT_SECRET, MockOidcProvider, NONCE, PKCE_VERIFIER, SUBJECT};

/// The mock stands in for Google. Which slot it occupies is arbitrary — the
/// code path under test is the shared OIDC one — but it has to be a slot whose
/// issuer comes from configuration rather than being hardcoded to the real
/// provider's URL.
const UNDER_TEST: ProviderKind = ProviderKind::Google;

/// Builds a registry whose Google provider is the mock, by way of the ordinary
/// configuration type. Nothing here reaches inside the provider layer: the
/// tests exercise the same construction path a deployment does, so a change
/// that breaks discovery or JWKS fetching breaks these tests too.
async fn registry_for(mock: &MockOidcProvider) -> ProviderRegistry {
    let config: AuthConfig = serde_json::from_value(json!({
        "base_url": "http://127.0.0.1:8080/",
        "providers": {
            "google": {
                "client_id": CLIENT_ID,
                "client_secret": CLIENT_SECRET,
                "issuer": mock.issuer().as_str(),
            }
        }
    }))
    .expect("the mock's auth configuration should deserialize");

    ProviderRegistry::from_config(&config).await
}

/// Requires successful provider discovery before testing token rejection.
#[track_caller]
fn provider(registry: &ProviderRegistry) -> &dyn AuthProvider {
    registry.get(UNDER_TEST).expect(
        "the mock provider is missing from the registry, which means discovery or JWKS \
         fetching against it failed; until that is fixed no assertion in this file means \
         anything",
    )
}

/// Asserts a token verification error, rejecting setup or transport errors
/// as evidence of a successful verification test.
#[track_caller]
fn assert_rejected(result: Result<VerifiedIdentity, AuthError>, what: &str) {
    match result {
        Ok(identity) => panic!(
            "{what} was ACCEPTED, yielding subject {:?}. This is the failure mode the phase \
             exists to prevent.",
            identity.subject
        ),
        Err(AuthError::TokenVerification(_)) => {}
        Err(other) => panic!(
            "{what} was rejected, but as `{other:?}` rather than a token verification failure. \
             The mock answered the exchange with HTTP 200 and a well-formed token response, so \
             the only thing left to fail is verification; any other variant means the test did \
             not reach the check it is about."
        ),
    }
}

// ---------------------------------------------------------------------------
// A self-check on the crafting machinery.
//
// Everything below this section is an assertion about the *service*. This one
// is an assertion about the *mock*, and it exists because the mock is the only
// thing standing behind every other test in the file: if it served a malformed
// discovery document, or published a key that did not correspond to the one it
// signs with, then every rejection test would pass without the service ever
// having verified anything. It also needs no provider implementation, so it
// verifies that the mock provider can issue a token accepted by the verifier.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_mock_publishes_the_key_it_signs_with_and_only_that_key() {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use openidconnect::JsonWebKey;
    use openidconnect::core::{CoreJsonWebKey, CoreJwsSigningAlgorithm};

    let mock = MockOidcProvider::start().await;

    let discovery: serde_json::Value =
        reqwest::get(format!("{}.well-known/openid-configuration", mock.issuer()))
            .await
            .expect("the discovery endpoint should answer")
            .json()
            .await
            .expect("discovery should be JSON");

    // Discovery is rejected outright if these two differ, so an issuer mismatch
    // would disable the provider rather than fail a token check — and every
    // rejection test would then pass for the wrong reason.
    // Compared against the issuer *identifier*, not `issuer().as_str()`: `Url`
    // normalizes a bare origin to carry a trailing slash, real providers
    // publish it without one, and the service trims the configured side to
    // match. Asserting against the un-trimmed form is what let a one-character
    // mismatch disable the provider while this self-check still passed.
    assert_eq!(discovery["issuer"], json!(mock.issuer_identifier()));
    assert_eq!(
        discovery["id_token_signing_alg_values_supported"],
        json!(["RS256"])
    );

    let jwks: serde_json::Value = reqwest::get(
        discovery["jwks_uri"]
            .as_str()
            .expect("discovery should advertise a jwks_uri"),
    )
    .await
    .expect("the JWKS endpoint should answer")
    .json()
    .await
    .expect("the JWKS should be JSON");

    let key: CoreJsonWebKey = serde_json::from_value(jwks["keys"][0].clone())
        .expect("the published key should deserialize as a JWK");

    let verify = |token: String| {
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "a JWS has three parts");
        let signature = URL_SAFE_NO_PAD
            .decode(parts[2])
            .expect("the signature should be base64url");
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        key.verify_signature(
            &CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            signing_input.as_bytes(),
            &signature,
        )
    };

    assert!(
        verify(mock.token().encode()).is_ok(),
        "a default token must verify against the published key, or the happy-path test below \
         is asserting nothing"
    );
    assert!(
        verify(mock.token().signed_by_a_different_key().encode()).is_err(),
        "the forged token must NOT verify against the published key; if it does, the two \
         keypairs are the same and the wrong-key test is vacuous"
    );
}

// ---------------------------------------------------------------------------
// The control. Without it, every rejection below could be explained by the
// happy path being broken.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_correctly_formed_token_verifies() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock.token().issue();
    let identity = provider(&registry)
        .exchange(&code, PKCE_VERIFIER, NONCE)
        .await
        .expect("a token with correct signature, issuer, audience, expiry and nonce must verify");

    assert_eq!(identity.provider, UNDER_TEST);
    assert_eq!(
        identity.subject, SUBJECT,
        "the subject must come from the token's `sub` claim, unmodified"
    );
    assert!(
        mock.jwks_requests() >= 1,
        "the signature cannot have been checked without fetching the key set"
    );
}

// ---------------------------------------------------------------------------
// Signature. The claims are the truth only if the signature says they are.
// ---------------------------------------------------------------------------

/// The token claims the published `kid`, so the verifier finds a key and has to
/// reject on the signature itself. Anyone can mint this: the header and claims
/// are public, and the only thing standing between an attacker and an arbitrary
/// `sub` is the signature check.
#[tokio::test]
async fn a_token_signed_by_a_different_key_is_rejected() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock
        .token()
        .signed_by_a_different_key()
        .subject("attacker-chosen-subject")
        .issue();

    assert_rejected(
        provider(&registry)
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token signed by a key that is not in the JWKS, bearing the published kid",
    );
}

/// The same forgery wearing an unknown `kid`, which is indistinguishable from a
/// legitimate key rotation. Target 6 requires a refetch on an unknown `kid`;
/// the refetch must not turn into acceptance, and it must not be unbounded —
/// otherwise made-up `kid`s become a way to point our traffic at the provider.
#[tokio::test]
async fn a_token_bearing_an_unknown_kid_is_rejected_without_hammering_the_jwks() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock.token().signed_by_an_unknown_key().issue();

    assert_rejected(
        provider(&registry)
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token signed by an unpublished key and bearing an unknown kid",
    );
    assert!(
        mock.jwks_requests() <= 3,
        "an unknown kid triggered {} JWKS fetches; refetching must be rate-limited so that \
         bogus kids cannot be used to drive traffic at the provider",
        mock.jwks_requests()
    );
}

// ---------------------------------------------------------------------------
// Audience and issuer. A perfectly valid token can still be somebody else's.
// ---------------------------------------------------------------------------

/// Correctly signed by the real provider, unexpired — and issued to a different
/// client. Every other relying party of the same provider holds tokens like
/// this for our users; accepting them hands each of them our accounts.
#[tokio::test]
async fn a_token_for_a_different_audience_is_rejected() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock.token().audience("some-other-relying-party").issue();

    assert_rejected(
        provider(&registry)
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token whose `aud` is another client",
    );
}

/// `iss` is what binds the subject namespace to a provider. Two providers can
/// hand out the same `sub`, so a token accepted from the wrong issuer collapses
/// two different people onto one identity row.
#[tokio::test]
async fn a_token_from_a_different_issuer_is_rejected() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock
        .token()
        .issuer("https://accounts.example.invalid/")
        .issue();

    assert_rejected(
        provider(&registry)
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token whose `iss` is not the provider discovery was performed against",
    );
}

// ---------------------------------------------------------------------------
// Expiry.
// ---------------------------------------------------------------------------

/// An hour stale, so this is about `exp` being checked at all rather than about
/// how much clock skew is tolerated. Without the check a token is a bearer
/// credential for ever, and the provider's revocation means nothing to us.
#[tokio::test]
async fn an_expired_token_is_rejected() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock.token().expired().issue();

    assert_rejected(
        provider(&registry)
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token that expired an hour ago",
    );
}

// ---------------------------------------------------------------------------
// Nonce. The replay defense, and the one most easily left out, because
// everything works without it.
// ---------------------------------------------------------------------------

/// The token is valid in every other respect but answers a different
/// authorization attempt. Nonce is what stops a token captured from one flow
/// being replayed into another.
#[tokio::test]
async fn a_token_whose_nonce_does_not_match_the_attempt_is_rejected() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock.token().nonce("nonce-from-some-other-attempt").issue();

    assert_rejected(
        provider(&registry)
            // The attempt's nonce, which the token does not carry.
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token carrying a nonce from a different attempt",
    );
}

/// A token missing nonce is rejected even when all other claims are valid.
#[tokio::test]
async fn a_token_with_no_nonce_at_all_is_rejected() {
    let mock = MockOidcProvider::start().await;
    let registry = registry_for(&mock).await;

    let code = mock.token().without_nonce().issue();

    assert_rejected(
        provider(&registry)
            .exchange(&code, PKCE_VERIFIER, NONCE)
            .await,
        "a token with no `nonce` claim, against an attempt that has one",
    );
}
