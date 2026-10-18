use super::*;
use crate::config::{AppleProviderConfig, AuthConfig, OidcProviderConfig, ProvidersConfig};
use secrecy::SecretString;

use crate::test_support as support;

const APPLE_TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg5XYNJMqUWV4t8Ive\n\
        aMzdW9Hywi9Gm8seX4KiD21HUhWhRANCAATtgsS2FtzL4WAKAi3QEq+IOOigdivH\n\
        EwyiPEKo7DuSlwxiTqRmG18C/R+nl1hA1RBdGaijKfRjoQO9hyxWj7kq\n\
        -----END PRIVATE KEY-----\n";

#[test]
fn a_deployment_with_no_credentials_still_has_a_registry() {
    // The state a fresh checkout starts in, and the reason `from_config`
    // does not return `Err` when nothing is configured.
    let registry = ProviderRegistry::empty();

    assert!(registry.configured().is_empty());
    for kind in DISPLAY_ORDER {
        assert!(registry.get(kind).is_none());
    }
}

#[tokio::test]
async fn configured_apple_provider_is_discovered_and_registered() {
    let mock = support::MockOidcProvider::start().await;
    let config = AuthConfig {
        initial_admins: Default::default(),
        base_url: url::Url::parse("http://pinvoyager.example").expect("valid test fixture"),
        session: Default::default(),
        providers: ProvidersConfig {
            apple: Some(AppleProviderConfig {
                client_id: support::CLIENT_ID.to_owned(),
                team_id: "ABCDE12345".to_owned(),
                key_id: "KEYID67890".to_owned(),
                private_key_pem: SecretString::from(APPLE_TEST_KEY),
            }),
            ..ProvidersConfig::default()
        },
        dev_login: false,
    };

    let registry =
        ProviderRegistry::from_config_with_apple_issuer(&config, mock.issuer().as_str()).await;

    assert_eq!(registry.configured(), vec![ProviderKind::Apple]);
    assert_eq!(
        registry
            .get(ProviderKind::Apple)
            .expect("valid test fixture")
            .kind(),
        ProviderKind::Apple
    );
}

#[tokio::test]
async fn every_configured_provider_is_discovered_registered_and_authorizable() {
    let mock = support::MockOidcProvider::start().await;
    let config = AuthConfig {
        initial_admins: Default::default(),
        base_url: url::Url::parse("http://pinvoyager.example").expect("valid test fixture"),
        session: Default::default(),
        providers: ProvidersConfig {
            google: Some(OidcProviderConfig {
                client_id: support::CLIENT_ID.to_owned(),
                client_secret: SecretString::from(support::CLIENT_SECRET),
                issuer: mock.issuer().clone(),
            }),
            microsoft: Some(OidcProviderConfig {
                client_id: support::CLIENT_ID.to_owned(),
                client_secret: SecretString::from(support::CLIENT_SECRET),
                issuer: mock.issuer().clone(),
            }),
            apple: Some(AppleProviderConfig {
                client_id: support::CLIENT_ID.to_owned(),
                team_id: "ABCDE12345".to_owned(),
                key_id: "KEYID67890".to_owned(),
                private_key_pem: SecretString::from(APPLE_TEST_KEY),
            }),
        },
        dev_login: false,
    };

    let registry =
        ProviderRegistry::from_config_with_apple_issuer(&config, mock.issuer().as_str()).await;

    assert_eq!(
        registry.configured(),
        vec![
            ProviderKind::Google,
            ProviderKind::Microsoft,
            ProviderKind::Apple,
        ]
    );
    for kind in registry.configured() {
        let provider = registry.get(kind).expect("configured provider is present");
        assert_eq!(provider.kind(), kind);
        assert!(provider.authorize().is_ok());
    }
}

#[tokio::test]
async fn a_provider_that_fails_startup_discovery_is_disabled_without_stopping_startup() {
    let config = AuthConfig {
        initial_admins: Default::default(),
        base_url: url::Url::parse("http://pinvoyager.example").expect("valid test fixture"),
        session: Default::default(),
        providers: ProvidersConfig {
            apple: Some(AppleProviderConfig {
                client_id: "service.example".to_owned(),
                team_id: "ABCDE12345".to_owned(),
                key_id: "KEYID67890".to_owned(),
                private_key_pem: SecretString::from("not a private key"),
            }),
            ..ProvidersConfig::default()
        },
        dev_login: false,
    };

    let registry =
        ProviderRegistry::from_config_with_apple_issuer(&config, "http://127.0.0.1:1/unreachable")
            .await;

    assert!(registry.configured().is_empty());
    assert!(registry.get(ProviderKind::Apple).is_none());
}

#[test]
fn every_protocol_provider_maps_to_its_storage_provider() {
    use crate::db::repo::identities::Provider as DbProvider;

    for (protocol, storage) in [
        (ProviderKind::Google, DbProvider::Google),
        (ProviderKind::Microsoft, DbProvider::Microsoft),
        (ProviderKind::Apple, DbProvider::Apple),
    ] {
        assert_eq!(DbProvider::from(protocol), storage);
    }
}

#[test]
fn removed_provider_is_not_recognized() {
    assert_eq!(ProviderKind::parse("facebook"), None);
    assert_eq!(DISPLAY_ORDER.len(), 3);
}
