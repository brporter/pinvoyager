//! Sign in with Apple.
//!
//! Apple is OIDC, and the verification path is the shared one in [`super::oidc`].
//! Three things are not shared, and each one reshapes something:
//!
//! 1. **The client secret is not a string, it is a JWT we mint.** Apple gives
//!    you a `.p8` ES256 private key and a key ID; the `client_secret` sent to
//!    the token endpoint is a short-lived JWT signed with it, valid for at most
//!    six months. So the secret has a lifetime, has to be cached, and has to be
//!    regenerated before it lapses — [`AppleClientSecret`].
//! 2. **`response_mode=form_post`.** Apple requires it whenever a scope beyond
//!    `openid` is requested, and returns the callback as an HTML form POST
//!    rather than a redirect with query parameters. That is why phase 3 has a
//!    `POST /auth/apple/callback` at all.
//! 3. **The name arrives once, in the POST body, and never again.** It is not in
//!    the ID token, it is not at any userinfo endpoint, and re-authorizing does
//!    not bring it back — only revoking the app in the user's Apple ID settings
//!    and starting over does. Absent is the *normal* case, not an error.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use openidconnect::{ClientSecret, Scope};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::auth::AuthError;
use crate::auth::provider::{AuthorizationRedirect, ProviderKind, VerifiedIdentity};
use crate::auth::providers::oidc::{ClientSecretSource, OidcCore};
use crate::config::AppleProviderConfig;

/// Apple's issuer. Unlike the other OIDC providers this is not configurable —
/// there is exactly one Apple, and a configurable issuer would only be a way to
/// point the flow somewhere else by mistake.
const APPLE_ISSUER: &str = "https://appleid.apple.com";

/// `aud` of the client-secret JWT. Fixed by Apple, and *not* the same thing as
/// the `aud` of the ID token, which is our Services ID.
const CLIENT_SECRET_AUDIENCE: &str = "https://appleid.apple.com";

/// How long a minted client secret is valid for.
///
/// Apple's ceiling is six months. Nothing is gained by going near it: the secret
/// is regenerated in-process for free, and a shorter life bounds how long a
/// leaked one is useful. Two weeks is comfortably longer than any deployment
/// gap and comfortably shorter than the limit.
const CLIENT_SECRET_LIFETIME: Duration = Duration::days(14);

/// Regenerate this long before expiry.
///
/// Without a margin the secret is renewed at the instant it stops working, so a
/// slow clock at Apple, a request already in flight, or the seconds between the
/// check and the token endpoint reading it all turn into a failed sign-in. The
/// failure mode of renewing early is one extra signature.
const CLIENT_SECRET_RENEW_BEFORE: Duration = Duration::hours(1);

pub(crate) struct AppleProvider {
    core: OidcCore,
}

impl AppleProvider {
    pub(crate) async fn discover(
        config: &AppleProviderConfig,
        base_url: &Url,
    ) -> Result<Self, AuthError> {
        let secret = Arc::new(AppleClientSecret::new(config)?);

        // Mint one now rather than at the first sign-in. A malformed `.p8`, a
        // key that is not actually ES256, or a missing signing backend is a
        // configuration error, and configuration errors belong at startup where
        // they disable this one provider with a log line — not in a user's
        // callback where they are a 500.
        secret.probe()?;

        let issuer = Url::parse(APPLE_ISSUER)
            .map_err(|err| AuthError::Configuration(format!("bad Apple issuer: {err}")))?;

        let core = OidcCore::discover(
            ProviderKind::Apple,
            &issuer,
            &config.client_id,
            secret,
            super::callback_uri(base_url, ProviderKind::Apple)?,
            // `name` is the only extra scope worth asking for. `email` is not
            // requested: this service does not store an email address, and Apple
            // would otherwise hand us a private-relay address we would have to
            // throw away.
            vec![Scope::new("name".to_owned())],
            // Required by Apple whenever a scope beyond `openid` is requested,
            // and the reason the Apple callback is a POST. Omitting it makes
            // Apple reject the authorization request outright.
            vec![("response_mode", "form_post")],
        )
        .await?;

        Ok(Self { core })
    }
}

#[async_trait::async_trait]
impl crate::auth::AuthProvider for AppleProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Apple
    }

    fn authorize(&self) -> Result<AuthorizationRedirect, AuthError> {
        self.core.authorize()
    }

    async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
        form_body: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        let mut identity = self.core.exchange(code, pkce_verifier, nonce).await?;

        // First authorization only, and nowhere else. If it is not taken from
        // this body it is gone permanently, so it is read before anything can
        // return early — but its absence is never a failure.
        if let Some(name) = display_name_from_form_body(form_body) {
            identity.display_name = Some(name);
        }

        Ok(identity)
    }
}

/// The `user` field Apple form-POSTs on first authorization.
///
/// Apple also puts an `email` (and `isPrivateEmail`) in here. They are
/// deliberately not fields on this struct: serde ignores what it is not asked
/// for, so the address is dropped at the parse boundary rather than existing in
/// a variable someone could later decide to store (D1).
#[derive(Debug, Deserialize)]
struct AppleUserPayload {
    name: Option<AppleName>,
}

#[derive(Debug, Deserialize)]
struct AppleName {
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
}

/// Digs the display name out of Apple's form-POST body.
///
/// Every failure here is `None`, never an error. A missing `user` field is the
/// ordinary case for every sign-in after the first, and even a malformed one is
/// not worth failing a sign-in over — the name is decoration, the subject is the
/// identity.
fn display_name_from_form_body(form_body: Option<&str>) -> Option<String> {
    let body = form_body?;

    let raw = url::form_urlencoded::parse(body.as_bytes())
        .find(|(key, _)| key == "user")
        .map(|(_, value)| value)?;

    let payload = match serde_json::from_str::<AppleUserPayload>(&raw) {
        Ok(payload) => payload,
        Err(err) => {
            // Worth a log: if Apple changes this shape, first authorizations
            // silently stop carrying names and nothing else would say so.
            tracing::warn!(error = %err, "could not parse Apple's `user` form field");
            return None;
        }
    };

    let name = payload.name?;
    let full = [name.first_name.as_deref(), name.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    (!full.is_empty()).then_some(full)
}

/// Mints and caches Apple's client-secret JWT.
struct AppleClientSecret {
    team_id: String,
    client_id: String,
    key_id: String,
    key: EncodingKey,
    cached: Mutex<Option<CachedSecret>>,
}

struct CachedSecret {
    token: String,
    expires_at: DateTime<Utc>,
}

/// The claims Apple requires. Any deviation is rejected as an invalid client.
#[derive(Debug, Serialize, Deserialize)]
struct ClientSecretClaims {
    /// The Apple Developer team ID.
    iss: String,
    iat: i64,
    exp: i64,
    /// Always Apple, never our own service.
    aud: String,
    /// The Services ID — the same value as the ID token's `aud`.
    sub: String,
}

impl AppleClientSecret {
    fn new(config: &AppleProviderConfig) -> Result<Self, AuthError> {
        // Parses and validates the PKCS#8 structure, so a mangled `.p8` (the
        // usual damage is a PEM whose newlines were eaten by an environment
        // variable) fails here rather than at the first signature.
        let key = EncodingKey::from_ec_pem(config.private_key_pem.expose_secret().as_bytes())
            .map_err(|err| {
                AuthError::Configuration(format!(
                    "auth.providers.apple.private_key_pem is not a usable EC private key: {err}"
                ))
            })?;

        Ok(Self {
            team_id: config.team_id.clone(),
            client_id: config.client_id.clone(),
            key_id: config.key_id.clone(),
            key,
            cached: Mutex::new(None),
        })
    }

    /// Mints one secret at startup so that a broken key is a startup problem.
    ///
    /// The `catch_unwind` covers a failure mode that cannot be typed away:
    /// `jsonwebtoken` 11 picks its signing backend from a *process-global*
    /// `CryptoProvider` selected by crate features, and if a future dependency
    /// change leaves none compiled in it does not return an error — `encode`
    /// panics. That panic would otherwise happen inside a user's callback, on
    /// every attempt, forever. Here it costs one call at startup and turns into
    /// the same outcome as any other provider misconfiguration: Apple disabled,
    /// reason logged, everything else still serving.
    fn probe(&self) -> Result<(), AuthError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.client_secret()))
            .map_err(|_| {
                AuthError::Configuration(
                    "signing the Apple client secret panicked — `jsonwebtoken` has no crypto \
                     backend compiled in (enable its `rust_crypto` or `aws_lc_rs` feature)"
                        .to_owned(),
                )
            })?
            .map(|_| ())
    }

    /// Returns the cached secret, minting a new one if there is none or the
    /// cached one is close enough to expiry to be risky.
    fn secret_at(&self, now: DateTime<Utc>) -> Result<String, AuthError> {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(current) = cached.as_ref()
            && now + CLIENT_SECRET_RENEW_BEFORE < current.expires_at
        {
            return Ok(current.token.clone());
        }

        let expires_at = now + CLIENT_SECRET_LIFETIME;
        let token = self.mint(now, expires_at)?;

        *cached = Some(CachedSecret {
            token: token.clone(),
            expires_at,
        });

        tracing::debug!(%expires_at, "minted a new Apple client secret");

        Ok(token)
    }

    fn mint(&self, now: DateTime<Utc>, expires_at: DateTime<Utc>) -> Result<String, AuthError> {
        let mut header = Header::new(Algorithm::ES256);
        // Apple looks the signing key up by `kid`; without it the token is
        // rejected even though the signature is perfectly good.
        header.kid = Some(self.key_id.clone());

        let claims = ClientSecretClaims {
            iss: self.team_id.clone(),
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
            aud: CLIENT_SECRET_AUDIENCE.to_owned(),
            sub: self.client_id.clone(),
        };

        // ES256 over a payload this small is microseconds and happens at most
        // once a fortnight per process, so it does not need `spawn_blocking`
        // (D8 is about the image pipeline, not about a single P-256 signature).
        jsonwebtoken::encode(&header, &claims, &self.key).map_err(|err| {
            AuthError::Configuration(format!("could not sign the Apple client secret: {err}"))
        })
    }
}

impl ClientSecretSource for AppleClientSecret {
    fn client_secret(&self) -> Result<ClientSecret, AuthError> {
        self.secret_at(Utc::now()).map(ClientSecret::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn first_authorization_name_is_taken_from_the_form_body() {
        let body = "code=abc&state=xyz&user=%7B%22name%22%3A%7B%22firstName%22%3A%22Ada%22%2C%22lastName%22%3A%22Lovelace%22%7D%2C%22email%22%3A%22ada%40example.com%22%7D";

        assert_eq!(
            display_name_from_form_body(Some(body)),
            Some("Ada Lovelace".to_owned())
        );
    }

    #[test]
    fn a_missing_name_is_normal_rather_than_an_error() {
        // Every Apple sign-in after the first looks like this.
        assert_eq!(
            display_name_from_form_body(Some("code=abc&state=xyz")),
            None
        );
        assert_eq!(display_name_from_form_body(None), None);
        // Present but useless, in the shapes Apple actually sends.
        assert_eq!(display_name_from_form_body(Some("user=%7B%7D")), None);
        assert_eq!(
            display_name_from_form_body(Some("user=%7B%22name%22%3A%7B%7D%7D")),
            None
        );
        // Malformed: logged, but never a failed sign-in.
        assert_eq!(display_name_from_form_body(Some("user=not-json")), None);
    }

    #[test]
    fn a_partial_name_is_still_a_name() {
        assert_eq!(
            display_name_from_form_body(Some(
                "user=%7B%22name%22%3A%7B%22firstName%22%3A%22Ada%22%7D%7D"
            )),
            Some("Ada".to_owned())
        );
    }
}
