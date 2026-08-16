//! A local OIDC provider that will lie on request.
//!
//! Phase 3's T3 criterion is "a token signed by the wrong key, with the wrong
//! `aud`, expired, or with a mismatched `nonce` is rejected in each case — test
//! with crafted tokens, not only live ones". No real provider will issue any of
//! those, so the only way to assert the rejection paths is to own the provider.
//! This is that provider: a real HTTP server on an ephemeral loopback port,
//! serving a real discovery document and a real JWKS, minting real RS256
//! signatures — and offering a knob for every property the verifier is supposed
//! to check.
//!
//! What it deliberately does **not** do is fake the verifier's inputs at a
//! lower level. The service under test performs discovery, fetches JWKS over
//! HTTP, and exchanges a code at a token endpoint; a mock that skipped any of
//! that would leave those steps untested and would pass even if the
//! implementation never fetched a key at all.
//!
//! ## Key material
//!
//! Both keypairs are generated at process start by shelling out to `openssl`.
//! Nothing is committed. A private key in a repository is a bad habit even when
//! it is only a test key — secret scanners cannot tell the difference, and the
//! resulting rotation fire drill costs somebody an afternoon.
//!
//! Two keys exist because "signed by the wrong key" is only a meaningful test
//! if the wrong key is a genuine RSA key producing a genuine signature that
//! simply does not verify against the published JWKS. The attacker key is never
//! served in `/jwks`.
//!
//! ## Deliberate omissions
//!
//! * Authorization codes are **not** single-use here. Replay of a code is T2's
//!   concern and belongs to the attempt/session layer, which enforces
//!   single-use on its own `state` record; making codes one-shot in the mock
//!   would only mean an implementation that retried an exchange failed for a
//!   confusing reason.
//! * The PKCE verifier and client credentials are accepted without checking.
//!   These tests are about ID token verification, and a mock that rejected them
//!   would produce failures that look identical to the ones under test.

// Not every helper here is used by every test binary that includes the module,
// and an unused-code warning is not a useful signal about test scaffolding.
#![allow(dead_code)]

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::Utc;
use openidconnect::core::{CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey};
use openidconnect::{JsonWebKeyId, PrivateSigningKey};
use serde_json::{Value, json};
use url::Url;

/// The client the mock believes it issued credentials to. Tokens carry this in
/// `aud` unless a test says otherwise.
pub const CLIENT_ID: &str = "pinvoyager-test-client";

/// Never checked by the mock — see the module note on deliberate omissions.
/// Present because the configuration type requires one.
pub const CLIENT_SECRET: &str = "pinvoyager-test-client-secret";

/// The nonce the mock stamps into every token by default, and the one a test
/// hands to `exchange` as the pending attempt's nonce. Defaulting to a *match*
/// means a mismatch is something a test has to ask for explicitly, so no test
/// can fail the nonce check by accident.
pub const NONCE: &str = "nonce-from-the-pending-attempt";

/// A syntactically valid RFC 7636 verifier (43 unreserved characters).
pub const PKCE_VERIFIER: &str = "vcJm5oDOvi5JzsO5AS9uTHFqLZfxqBRCQ0Vf8DPIWpM";

/// Default `sub`. Opaque, as the contract requires — never parsed.
pub const SUBJECT: &str = "mock-provider-subject-0001";

/// The `kid` published in `/jwks`.
const LEGITIMATE_KID: &str = "mock-key-1";

/// The `kid` of the key that is *not* published. A token bearing it should send
/// a correct implementation looking for an unknown key, and then to a rejection
/// — not to an unbounded stream of refetches.
const ATTACKER_KID: &str = "attacker-key-1";

// ---------------------------------------------------------------------------
// Key material, generated once per test binary.
// ---------------------------------------------------------------------------

struct Keys {
    /// Published in `/jwks`. Signs anything a test has not sabotaged.
    legitimate: CoreRsaPrivateSigningKey,
    /// Never published. Stands in for an attacker who can produce a
    /// well-formed, correctly-claimed, properly-signed token — with the wrong
    /// key. This is the token that a verifier which only *decodes* accepts.
    attacker: CoreRsaPrivateSigningKey,
}

/// RSA-2048 keygen is not free, so pay for it once per test binary rather than
/// once per test.
static KEYS: LazyLock<Keys> = LazyLock::new(|| Keys {
    legitimate: signing_key(LEGITIMATE_KID),
    attacker: signing_key(ATTACKER_KID),
});

/// The published key set. The attacker key is conspicuously absent.
static JWKS: LazyLock<Value> = LazyLock::new(|| {
    let mut key = serde_json::to_value(KEYS.legitimate.as_verification_key())
        .expect("a JSON Web Key should serialize");
    // `alg` is optional in RFC 7517 but every real provider publishes it, and
    // an implementation is entitled to use it to pick a verifier.
    if let Some(object) = key.as_object_mut() {
        object.insert("alg".to_owned(), json!("RS256"));
    }
    json!({ "keys": [key] })
});

fn signing_key(kid: &str) -> CoreRsaPrivateSigningKey {
    let pem = generate_rsa_private_key_pem();
    CoreRsaPrivateSigningKey::from_pem(&pem, Some(JsonWebKeyId::new(kid.to_owned())))
        .expect("the generated PEM should parse as a PKCS#1 RSA private key")
}

/// Generates a throwaway RSA key, in the PKCS#1 PEM form the signer accepts.
///
/// `openssl` is the only key generator reachable from this crate's dependency
/// set: `jsonwebtoken` is compiled with neither of its crypto backend features,
/// so it can neither generate nor sign, and the RSA implementation inside
/// `openidconnect` is private to it.
fn generate_rsa_private_key_pem() -> String {
    let output = Command::new("openssl")
        .args(["genrsa", "2048"])
        .output()
        .expect("`openssl` must be on PATH to generate a throwaway test key");
    assert!(
        output.status.success(),
        "openssl genrsa failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pem = String::from_utf8(output.stdout).expect("openssl should emit UTF-8 PEM");

    // OpenSSL 3 emits PKCS#8 from `genrsa`; LibreSSL (and OpenSSL 1.x) emit
    // PKCS#1. The signer only takes PKCS#1, so normalise rather than depend on
    // which one happens to be installed.
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        pem
    } else {
        pkcs8_pem_to_pkcs1_pem(&pem)
    }
}

/// Unwraps a PKCS#8 `PrivateKeyInfo` to the PKCS#1 `RSAPrivateKey` inside it.
///
/// `PrivateKeyInfo ::= SEQUENCE { version INTEGER, algorithm SEQUENCE,
/// privateKey OCTET STRING }` — and for RSA that octet string is exactly the
/// PKCS#1 DER, so the conversion is a walk, not a re-encode.
fn pkcs8_pem_to_pkcs1_pem(pem: &str) -> String {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = STANDARD
        .decode(body.trim())
        .expect("the PEM body should be base64");

    let (info, _) = der_tlv(&der, 0x30);
    let (_, after_version) = der_tlv(info, 0x02);
    let (_, after_algorithm) = der_tlv(after_version, 0x30);
    let (private_key, _) = der_tlv(after_algorithm, 0x04);

    let encoded = STANDARD.encode(private_key);
    let mut out = String::from("-----BEGIN RSA PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END RSA PRIVATE KEY-----\n");
    out
}

/// Splits one DER tag-length-value off the front of `input`, checking the tag.
/// Returns `(contents, remainder)`. Panics on anything unexpected: this parses
/// output we just generated, so a surprise means a broken assumption, and a
/// broken assumption should stop the test run rather than yield a subtly wrong
/// key.
fn der_tlv(input: &[u8], expected_tag: u8) -> (&[u8], &[u8]) {
    assert_eq!(
        input.first().copied(),
        Some(expected_tag),
        "unexpected DER tag while unwrapping the PKCS#8 key"
    );
    let first_length_byte = input[1];
    let (length, header_len) = if first_length_byte < 0x80 {
        (usize::from(first_length_byte), 2)
    } else {
        let count = usize::from(first_length_byte & 0x7f);
        let mut length = 0usize;
        for byte in &input[2..2 + count] {
            length = (length << 8) | usize::from(*byte);
        }
        (length, 2 + count)
    };
    (
        &input[header_len..header_len + length],
        &input[header_len + length..],
    )
}

// ---------------------------------------------------------------------------
// The server.
// ---------------------------------------------------------------------------

struct Minted {
    id_token: String,
    subject: String,
}

#[derive(Clone)]
struct MockState {
    issuer: Url,
    minted: Arc<Mutex<HashMap<String, Minted>>>,
    jwks_requests: Arc<AtomicUsize>,
}

/// A running OIDC provider on 127.0.0.1, with an ephemeral port so that
/// concurrently-running tests cannot collide on one.
pub struct MockOidcProvider {
    issuer: Url,
    state: MockState,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for MockOidcProvider {
    fn drop(&mut self) {
        // Each test gets its own provider; without this they accumulate for the
        // lifetime of the test binary.
        self.server.abort();
    }
}

impl MockOidcProvider {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral loopback port should succeed");
        let addr = listener
            .local_addr()
            .expect("the listener should report its address");

        // Parsing and re-serializing here means `issuer()` hands out exactly
        // the string the discovery document will contain. OIDC discovery
        // requires the two to match exactly, and a trailing-slash difference is
        // the classic way to fail that check.
        let issuer = Url::parse(&format!("http://{addr}")).expect("a loopback URL should parse");

        let state = MockState {
            issuer: issuer.clone(),
            minted: Arc::new(Mutex::new(HashMap::new())),
            jwks_requests: Arc::new(AtomicUsize::new(0)),
        };

        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/userinfo", get(userinfo))
            .with_state(state.clone());

        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            issuer,
            state,
            server,
        }
    }

    /// The issuer URL to configure the service with, and the base for joining
    /// endpoint paths onto.
    pub fn issuer(&self) -> &Url {
        &self.issuer
    }

    /// The issuer *identifier* — the exact string the discovery document and
    /// every ID token's `iss` claim carry.
    ///
    /// Not the same as `issuer().as_str()`: `Url` gives a bare origin a
    /// trailing slash and no real provider publishes it that way, so the
    /// service trims it off the configured side. This is the trimmed form, and
    /// it is what any assertion about the published issuer should compare
    /// against.
    pub fn issuer_identifier(&self) -> &str {
        issuer_identifier(&self.issuer)
    }

    /// How many times `/jwks` has been fetched. Key rotation handling (target 6)
    /// is about refetching on an unknown `kid` *and* not doing so without
    /// bound; this is how a test can see either.
    pub fn jwks_requests(&self) -> usize {
        self.state.jwks_requests.load(Ordering::Relaxed)
    }

    /// Starts building an ID token. Every default is correct; a test changes
    /// exactly the one property it is about, so a failure can only mean that
    /// property was checked — or was not.
    pub fn token(&self) -> TokenBuilder<'_> {
        let now = Utc::now().timestamp();
        TokenBuilder {
            mock: self,
            kid: LEGITIMATE_KID.to_owned(),
            key: &KEYS.legitimate,
            claims: json!({
                // Same trimming as the discovery document: the ID token's `iss`
                // is checked against the issuer identifier, so a slash here
                // would make every correctly-formed token fail for the wrong
                // reason.
                "iss": issuer_identifier(&self.issuer),
                "aud": CLIENT_ID,
                "sub": SUBJECT,
                "iat": now,
                "exp": now + 300,
                "nonce": NONCE,
            }),
        }
    }
}

/// The issuer identifier as a real provider emits it.
///
/// `Url` normalises a bare origin to carry a trailing slash — `http://host:port`
/// becomes `http://host:port/` — but no real provider publishes it that way.
/// Google's document says `https://accounts.google.com`, and OIDC compares the
/// configured issuer against the published one for *exact* equality, so the
/// service trims the slash off the configured side to match.
///
/// The mock therefore has to publish the trimmed form too. Emitting the
/// normalised one made discovery fail on a one-character difference, the
/// provider was silently disabled, and every rejection test in this file
/// "passed" by never reaching its assertion.
fn issuer_identifier(issuer: &Url) -> &str {
    issuer.as_str().trim_end_matches('/')
}

async fn discovery(State(state): State<MockState>) -> Json<Value> {
    Json(json!({
        // Must be byte-identical to the configured issuer, or discovery is
        // rejected before any token is ever minted.
        "issuer": issuer_identifier(&state.issuer),
        "authorization_endpoint": endpoint(&state.issuer, "authorize"),
        "token_endpoint": endpoint(&state.issuer, "token"),
        "userinfo_endpoint": endpoint(&state.issuer, "userinfo"),
        "jwks_uri": endpoint(&state.issuer, "jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "scopes_supported": ["openid", "profile", "email"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "claims_supported": ["iss", "aud", "sub", "iat", "exp", "nonce", "name", "picture"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

fn endpoint(issuer: &Url, path: &str) -> String {
    issuer
        .join(path)
        .expect("joining a path onto the issuer should succeed")
        .to_string()
}

async fn jwks(State(state): State<MockState>) -> Json<Value> {
    state.jwks_requests.fetch_add(1, Ordering::Relaxed);
    Json(JWKS.clone())
}

/// Present only because discovery must advertise it. Nothing follows the
/// redirect in these tests; the browser leg is T2's business.
async fn authorize() -> &'static str {
    "the mock provider does not render a consent screen"
}

async fn token(State(state): State<MockState>, body: String) -> Response {
    let params: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();

    let Some(code) = params.get("code") else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "no code");
    };

    let id_token = {
        let minted = state
            .minted
            .lock()
            .expect("the mint should not be poisoned");
        minted.get(code).map(|entry| entry.id_token.clone())
    };

    let Some(id_token) = id_token else {
        // What a real provider says to an unknown or already-redeemed code.
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "unknown code");
    };

    Json(json!({
        // Reusing the code as the access token keeps /userinfo answerable
        // without a second table. D1 forbids the *service* from storing this;
        // the provider side is allowed to remember what it issued.
        "access_token": code,
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": "openid profile",
        "id_token": id_token,
    }))
    .into_response()
}

/// Some implementations follow the ID token with a userinfo call. Answering it
/// consistently means a test failure points at the token under test rather than
/// at a 404 from the mock.
async fn userinfo(State(state): State<MockState>, headers: axum::http::HeaderMap) -> Response {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned();

    let subject = {
        let minted = state
            .minted
            .lock()
            .expect("the mint should not be poisoned");
        minted.get(&token).map(|entry| entry.subject.clone())
    };

    match subject {
        Some(subject) => Json(json!({ "sub": subject })).into_response(),
        None => oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "unknown access token",
        ),
    }
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Token crafting.
// ---------------------------------------------------------------------------

/// Builds one ID token. The defaults produce a token that must verify; each
/// method below breaks exactly one property.
pub struct TokenBuilder<'a> {
    mock: &'a MockOidcProvider,
    kid: String,
    key: &'static CoreRsaPrivateSigningKey,
    claims: Value,
}

impl TokenBuilder<'_> {
    /// Claims a different `iss` than the one discovery was performed against.
    pub fn issuer(mut self, issuer: &str) -> Self {
        self.set("iss", json!(issuer));
        self
    }

    /// Issues to a different client. A token minted for someone else's client
    /// ID is a valid, correctly-signed, unexpired token — accepting it lets any
    /// other relying party of the same provider impersonate our users.
    pub fn audience(mut self, audience: &str) -> Self {
        self.set("aud", json!(audience));
        self
    }

    pub fn subject(mut self, subject: &str) -> Self {
        self.set("sub", json!(subject));
        self
    }

    pub fn nonce(mut self, nonce: &str) -> Self {
        self.set("nonce", json!(nonce));
        self
    }

    /// Drops the claim entirely, rather than setting it to something wrong.
    /// An implementation that compares the nonce only when one is present
    /// passes the mismatch test and is still replayable.
    pub fn without_nonce(mut self) -> Self {
        self.remove("nonce");
        self
    }

    /// Well past any defensible clock skew, so the assertion is about `exp`
    /// being checked at all rather than about the size of a leeway window.
    pub fn expired(mut self) -> Self {
        let now = Utc::now().timestamp();
        self.set("iat", json!(now - 7200));
        self.set("exp", json!(now - 3600));
        self
    }

    /// Signs with a key that is not in the JWKS, while still claiming the
    /// published `kid`. This is the sharper of the two wrong-key cases: the
    /// verifier finds a key, so it cannot reject on lookup — it has to actually
    /// check the signature.
    pub fn signed_by_a_different_key(mut self) -> Self {
        self.key = &KEYS.attacker;
        self
    }

    /// Signs with the unpublished key *and* claims its unpublished `kid`, which
    /// is what a rotation looks like from the outside. Correct handling is a
    /// bounded refetch followed by rejection.
    pub fn signed_by_an_unknown_key(mut self) -> Self {
        self.key = &KEYS.attacker;
        self.kid = ATTACKER_KID.to_owned();
        self
    }

    /// Escape hatch for a claim the builder has no opinion about.
    pub fn claim(mut self, name: &str, value: Value) -> Self {
        self.set(name, value);
        self
    }

    fn set(&mut self, name: &str, value: Value) {
        self.claims
            .as_object_mut()
            .expect("claims are a JSON object")
            .insert(name.to_owned(), value);
    }

    fn remove(&mut self, name: &str) {
        self.claims
            .as_object_mut()
            .expect("claims are a JSON object")
            .remove(name);
    }

    /// The signed JWT itself.
    pub fn encode(&self) -> String {
        let header = json!({ "alg": "RS256", "typ": "JWT", "kid": self.kid });
        let signing_input = format!("{}.{}", base64_json(&header), base64_json(&self.claims));
        let signature = self
            .key
            .sign(
                &CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
                signing_input.as_bytes(),
            )
            .expect("signing with a generated RSA key should succeed");
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    /// Registers the token against a fresh authorization code and returns the
    /// code — which is what the service under test is given, since it reaches
    /// the ID token through a real code exchange rather than being handed one.
    pub fn issue(self) -> String {
        let code = format!("code-{}", uuid::Uuid::now_v7());
        let subject = self
            .claims
            .get("sub")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let id_token = self.encode();

        self.mock
            .state
            .minted
            .lock()
            .expect("the mint should not be poisoned")
            .insert(code.clone(), Minted { id_token, subject });

        code
    }
}

fn base64_json(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).expect("JSON should serialize"))
}
