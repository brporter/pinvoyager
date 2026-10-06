use super::*;

#[test]
fn callback_uris_are_built_the_same_way_whatever_the_base_url_looks_like() {
    // A trailing slash on `auth.base_url` must not produce `//auth/...`:
    // the provider compares this string byte for byte against what is
    // registered in its console and rejects anything else.
    for base in ["https://pinvoyager.example", "https://pinvoyager.example/"] {
        let base = Url::parse(base).expect("test URL should parse");
        let uri = callback_uri(&base, ProviderKind::Google).expect("should build");
        assert_eq!(
            uri.as_str(),
            "https://pinvoyager.example/auth/google/callback"
        );
    }
}

#[test]
fn each_provider_gets_its_own_callback_path() {
    let base = Url::parse("http://127.0.0.1:8080").expect("test URL should parse");
    for kind in [
        ProviderKind::Google,
        ProviderKind::Microsoft,
        ProviderKind::Apple,
        ProviderKind::Facebook,
    ] {
        let uri = callback_uri(&base, kind).expect("should build");
        assert_eq!(
            uri.as_str(),
            format!("http://127.0.0.1:8080/auth/{}/callback", kind.as_str())
        );
    }
}

#[test]
fn a_base_url_without_a_hierarchical_path_is_rejected() {
    let base = Url::parse("mailto:owner@example.com").expect("mailto URL parses");

    assert!(matches!(
        callback_uri(&base, ProviderKind::Google),
        Err(AuthError::Configuration(_))
    ));
}
