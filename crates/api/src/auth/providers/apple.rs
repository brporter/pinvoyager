//! Sign in with Apple, using the shared OIDC verification path.
//!
//! Apple requires an ES256 JWT as the client secret. `AppleClientSecret`
//! caches that signature and renews it before expiry. Authorization uses a
//! form-POST callback.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use openidconnect::ClientSecret;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::auth::AuthError;
use crate::auth::provider::{AuthorizationRedirect, ProviderKind, VerifiedIdentity};
use crate::auth::providers::oidc::{ClientSecretSource, OidcCore};
use crate::config::AppleProviderConfig;

/// Fixed issuer for Sign in with Apple.
pub(crate) const APPLE_ISSUER: &str = "https://appleid.apple.com";

/// `aud` of the client-secret JWT. Fixed by Apple, and *not* the same thing as
/// the `aud` of the ID token, which is our Services ID.
const CLIENT_SECRET_AUDIENCE: &str = "https://appleid.apple.com";

/// Lifetime of a generated client-secret JWT: two weeks.
const CLIENT_SECRET_LIFETIME: Duration = Duration::days(14);

/// Renew the cached secret this far ahead of its expiration.
const CLIENT_SECRET_RENEW_BEFORE: Duration = Duration::hours(1);

pub(crate) struct AppleProvider {
    core: OidcCore,
}

impl AppleProvider {
    pub(crate) async fn discover(
        config: &AppleProviderConfig,
        base_url: &Url,
        issuer: &str,
    ) -> Result<Self, AuthError> {
        let secret = Arc::new(AppleClientSecret::new(config)?);

        // Validate signing at startup so a broken key disables Apple before any callback.
        secret.probe()?;

        let issuer = Url::parse(issuer)
            .map_err(|err| AuthError::Configuration(format!("bad Apple issuer: {err}")))?;

        let core = OidcCore::discover(
            ProviderKind::Apple,
            &issuer,
            &config.client_id,
            secret,
            super::callback_uri(base_url, ProviderKind::Apple)?,
            vec![],
            // Keep the callback transport consistent with the registered Apple flow.
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
    ) -> Result<VerifiedIdentity, AuthError> {
        self.core.exchange(code, pkce_verifier, nonce).await
    }
    async fn exchange_for_deletion(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
        expected_subject: &str,
    ) -> Result<VerifiedIdentity, AuthError> {
        self.core
            .exchange_for_deletion(code, verifier, nonce, expected_subject)
            .await
    }
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

    /// Mints a secret to validate the signing key and backend at startup.
    /// Converts signing errors and encoder panics to an authentication error.
    fn probe(&self) -> Result<(), AuthError> {
        catch_secret_probe(|| self.client_secret())
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

        // Sign synchronously; secrets are cached and renewed near expiration.
        jsonwebtoken::encode(&header, &claims, &self.key).map_err(|err| {
            AuthError::Configuration(format!("could not sign the Apple client secret: {err}"))
        })
    }
}

fn catch_secret_probe(
    generate: impl FnOnce() -> Result<ClientSecret, AuthError>,
) -> Result<(), AuthError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(generate))
        .map_err(|_| {
            AuthError::Configuration(
                "signing the Apple client secret panicked — `jsonwebtoken` has no crypto \
                 backend compiled in (enable its `rust_crypto` or `aws_lc_rs` feature)"
                    .to_owned(),
            )
        })?
        .map(|_| ())
}

impl ClientSecretSource for AppleClientSecret {
    fn client_secret(&self) -> Result<ClientSecret, AuthError> {
        self.secret_at(Utc::now()).map(ClientSecret::new)
    }
}

#[cfg(test)]
#[path = "apple_tests.rs"]
mod tests;
