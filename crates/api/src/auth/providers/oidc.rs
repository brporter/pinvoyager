//! The OIDC machinery shared by Google, Microsoft and Apple.
//!
//! `OidcCore::discover` fetches endpoints, supported algorithms, and signing keys
//! at startup. Endpoints and algorithms remain cached for the process lifetime.
//! An unknown signing key triggers a JWKS refresh and one verification retry;
//! refresh attempts are at least five minutes apart.

use std::sync::{Arc, RwLock};

use base64::Engine as _;
use serde_json::Value;
use std::time::{Duration, Instant};

use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreProviderMetadata,
};
use openidconnect::{
    AuthType, AuthorizationCode, ClaimsVerificationError, ClientId, ClientSecret, CsrfToken,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, SignatureVerificationError,
    TokenResponse,
};
use secrecy::{ExposeSecret, SecretString};
use url::Url;

use crate::auth::AuthError;
use crate::auth::provider::{AuthorizationRedirect, ProviderKind, VerifiedIdentity};
use crate::auth::providers::provider_http_client;

/// The endpoint states a client built from discovery ends up in: authorization
/// URL known, token and userinfo URLs known *if the provider published them*,
/// and no device/introspection/revocation endpoints configured. Spelling it out
/// once keeps the generic soup out of every signature below.
type DiscoveredClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// Minimum interval between attempts to refresh signing keys after an unknown
/// key ID. The initial discovery fetch also starts this interval.
const JWKS_MIN_REFETCH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Where the `client_secret` presented at the token endpoint comes from.
///
/// Google and Microsoft use a static string. Apple's is a JWT that this
/// service mints and that expires, so it cannot be captured once at startup —
/// hence the indirection rather than a `ClientSecret` field.
pub(crate) trait ClientSecretSource: Send + Sync {
    fn client_secret(&self) -> Result<ClientSecret, AuthError>;
}

/// Configured provider client secret stored in the OAuth client's redacted type.
pub(crate) struct StaticClientSecret(ClientSecret);

impl StaticClientSecret {
    pub(crate) fn new(secret: &SecretString) -> Self {
        Self(ClientSecret::new(secret.expose_secret().to_owned()))
    }
}

impl ClientSecretSource for StaticClientSecret {
    fn client_secret(&self) -> Result<ClientSecret, AuthError> {
        Ok(self.0.clone())
    }
}

/// Discovery output, plus what is needed to keep the key set current.
struct CachedMetadata {
    metadata: CoreProviderMetadata,
    /// Preserve Microsoft signing-key issuer restrictions, which CoreJsonWebKey ignores.
    microsoft_jwks: Option<Value>,
    /// When the JWKS in `metadata` was last fetched, for the refetch floor.
    /// Set at discovery, so a provider cannot be hit again immediately after
    /// startup either.
    jwks_fetched_at: Instant,
}

/// One discovered OIDC provider.
pub(crate) struct OidcCore {
    kind: ProviderKind,
    client_id: ClientId,
    secret: Arc<dyn ClientSecretSource>,
    redirect_uri: RedirectUrl,
    revocation_url: Url,
    scopes: Vec<Scope>,
    /// Authorization-request parameters the OIDC spec does not model. Apple's
    /// `response_mode=form_post` is the only user today.
    extra_authorize_params: Vec<(&'static str, &'static str)>,
    http: reqwest::Client,
    metadata: RwLock<CachedMetadata>,
}

/// Converts the configured issuer to the spelling used for discovery.
///
/// Removes trailing slashes to match discovery issuer spelling, including
/// Google's. Returns a configuration error unless the URL uses HTTP or HTTPS
/// and has a host.
fn issuer_url(issuer: &Url) -> Result<IssuerUrl, AuthError> {
    if !matches!(issuer.scheme(), "http" | "https") || issuer.host_str().is_none() {
        return Err(AuthError::Configuration(
            "invalid issuer URL: expected an HTTP or HTTPS URL with a host".to_owned(),
        ));
    }

    // Removing trailing slashes from an already parsed HTTP URL cannot
    // invalidate it. Preserve the raw spelling required for issuer comparison.
    Ok(
        IssuerUrl::new(issuer.as_str().trim_end_matches('/').to_owned())
            .expect("a normalized HTTP issuer remains a valid URL"),
    )
}

const MICROSOFT_ISSUER_TEMPLATE: &str = "https://login.microsoftonline.com/{tenantid}/v2.0";

fn microsoft_common(kind: ProviderKind, issuer: &Url) -> bool {
    kind == ProviderKind::Microsoft
        && issuer.as_str().trim_end_matches('/') == "https://login.microsoftonline.com/common/v2.0"
}

async fn fetch_json(http: &reqwest::Client, url: &str) -> Result<Value, AuthError> {
    http.get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| AuthError::Provider(format!("OIDC metadata request failed: {err}")))?
        .json()
        .await
        .map_err(|err| AuthError::Provider(format!("invalid OIDC metadata: {err}")))
}

/// Fetch only from the configured authority. Never follow a URL supplied by a token.
async fn discover_microsoft_metadata(
    issuer: &IssuerUrl,
    http: &reqwest::Client,
) -> Result<(CoreProviderMetadata, Value), AuthError> {
    let mut document = fetch_json(
        http,
        &format!("{}/.well-known/openid-configuration", issuer.as_str()),
    )
    .await?;
    if document["issuer"].as_str() != Some(MICROSOFT_ISSUER_TEMPLATE) {
        return Err(AuthError::Configuration(
            "unexpected Microsoft discovery issuer".into(),
        ));
    }
    // The common authority is used for authorization and code exchange. Verification
    // replaces it with the concrete tenant issuer after checking the token's tid.
    document["issuer"] = Value::String(issuer.as_str().to_owned());
    let metadata: CoreProviderMetadata = serde_json::from_value(document)
        .map_err(|err| AuthError::Configuration(format!("invalid Microsoft discovery: {err}")))?;
    let keys = fetch_json(http, metadata.jwks_uri().as_str()).await?;
    let jwks = serde_json::from_value(keys.clone())
        .map_err(|err| AuthError::Configuration(format!("invalid Microsoft JWKS: {err}")))?;
    Ok((metadata.set_jwks(jwks), keys))
}

/// Decode tid only to choose verification inputs. No identity is trusted until
/// the library checks the signature, exact issuer, audience, expiry, and nonce.
fn microsoft_token_issuer(token: &str) -> Result<IssuerUrl, AuthError> {
    let invalid = || AuthError::TokenVerification("invalid Microsoft tenant ID".into());
    let payload = token.split('.').nth(1).ok_or_else(invalid)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| invalid())?;
    let claims: Value = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    let tenant = claims["tid"].as_str().ok_or_else(invalid)?;
    let uuid = uuid::Uuid::parse_str(tenant).map_err(|_| invalid())?;
    if uuid.hyphenated().to_string() != tenant {
        return Err(invalid());
    }
    IssuerUrl::new(MICROSOFT_ISSUER_TEMPLATE.replace("{tenantid}", tenant)).map_err(|_| invalid())
}

impl OidcCore {
    /// Fetches discovery metadata and signing keys for the initial cache.
    pub(crate) async fn discover(
        kind: ProviderKind,
        issuer: &Url,
        client_id: &str,
        secret: Arc<dyn ClientSecretSource>,
        redirect_uri: RedirectUrl,
        scopes: Vec<Scope>,
        extra_authorize_params: Vec<(&'static str, &'static str)>,
    ) -> Result<Self, AuthError> {
        let common = microsoft_common(kind, issuer);
        let issuer = issuer_url(issuer)?;
        let revocation_url = issuer
            .url()
            .join("/auth/revoke")
            .map_err(|error| AuthError::Configuration(error.to_string()))?;
        let http = provider_http_client()?;

        let (metadata, microsoft_jwks) = if common {
            let (metadata, keys) = discover_microsoft_metadata(&issuer, &http).await?;
            (metadata, Some(keys))
        } else {
            let metadata = CoreProviderMetadata::discover_async(issuer, &http)
                .await
                .map_err(|err| AuthError::Configuration(format!("OIDC discovery failed: {err}")))?;
            (metadata, None)
        };

        Ok(Self {
            kind,
            client_id: ClientId::new(client_id.to_owned()),
            revocation_url,
            secret,
            redirect_uri,
            scopes,
            extra_authorize_params,
            http,
            metadata: RwLock::new(CachedMetadata {
                metadata,
                microsoft_jwks,
                jwks_fetched_at: Instant::now(),
            }),
        })
    }

    /// Builds a client from cached metadata and the supplied secret without I/O.
    /// Each exchange uses current signing keys and, for Apple, a current secret.
    fn client(&self, secret: Option<ClientSecret>) -> DiscoveredClient {
        let metadata = {
            let cached = self
                .metadata
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cached.metadata.clone()
        };

        CoreClient::from_provider_metadata(metadata, self.client_id.clone(), secret)
            .set_redirect_uri(self.redirect_uri.clone())
            // `client_secret_post` is the one method all three of these
            // providers advertise: Apple supports *only* it, and Google and
            // Microsoft support it alongside HTTP Basic. `oauth2` defaults to
            // Basic, which Apple rejects.
            .set_auth_type(AuthType::RequestBody)
    }

    fn verification_client(&self, token: &str) -> Result<DiscoveredClient, AuthError> {
        let cached = self
            .metadata
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(keys) = &cached.microsoft_jwks else {
            drop(cached);
            return Ok(self.client(None));
        };
        let issuer = microsoft_token_issuer(token)?;
        let matching: Vec<_> = keys["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|key| {
                matches!(key["issuer"].as_str(), Some(scope)
                if scope == MICROSOFT_ISSUER_TEMPLATE || scope == issuer.as_str())
            })
            .cloned()
            .collect();
        let jwks =
            serde_json::from_value(serde_json::json!({"keys": matching})).map_err(|err| {
                AuthError::TokenVerification(format!("invalid Microsoft signing keys: {err}"))
            })?;
        let metadata = cached.metadata.clone().set_issuer(issuer).set_jwks(jwks);
        Ok(
            CoreClient::from_provider_metadata(metadata, self.client_id.clone(), None)
                .set_redirect_uri(self.redirect_uri.clone()),
        )
    }

    pub(crate) fn authorize(&self) -> Result<AuthorizationRedirect, AuthError> {
        // The secret plays no part in building an authorization URL, so it is
        // deliberately not requested here — for Apple that would mean signing a
        // JWT to put it in a URL that never carries it.
        let client = self.client(None);

        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();

        let mut request = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge);

        for scope in &self.scopes {
            request = request.add_scope(scope.clone());
        }
        for (name, value) in &self.extra_authorize_params {
            request = request.add_extra_param(*name, *value);
        }

        let (url, state, nonce) = request.url();

        Ok(AuthorizationRedirect {
            url,
            state: state.secret().to_owned(),
            nonce: nonce.secret().to_owned(),
            pkce_verifier: verifier.secret().to_owned(),
        })
    }

    /// Exchanges the code and verifies the ID token that comes back.
    pub(crate) async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<VerifiedIdentity, AuthError> {
        self.exchange_with_revocation(code, pkce_verifier, nonce, None)
            .await
    }

    pub(crate) async fn exchange_for_deletion(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
        expected_subject: &str,
    ) -> Result<VerifiedIdentity, AuthError> {
        self.exchange_with_revocation(code, verifier, nonce, Some(expected_subject))
            .await
    }

    async fn exchange_with_revocation(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
        revoke_subject: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        let client = self.client(Some(self.secret.client_secret()?));

        let token_response = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|err| {
                AuthError::Configuration(format!("provider published no token endpoint: {err}"))
            })?
            // PKCE: the provider recomputes the challenge from this verifier and
            // rejects the exchange unless it matches the one sent at `/start`,
            // which is what stops an intercepted code being redeemed by anyone
            // but us.
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
            .request_async(&self.http)
            .await
            .map_err(|err| AuthError::Provider(format!("code exchange failed: {err}")))?;

        // D1: `token_response` also carries an access token, and possibly a
        // refresh token. They remain in this function; deletion revokes a fresh
        // token after verifying the expected subject, then drops both.
        let id_token = token_response.id_token().ok_or_else(|| {
            AuthError::TokenVerification("provider returned no ID token".to_owned())
        })?;

        let nonce = Nonce::new(nonce.to_owned());

        // Everything D1 calls for happens inside this one call:
        //
        //   signature  against the JWKS from discovery, restricted to the
        //              algorithms in `id_token_signing_alg_values_supported`
        //              (set by `from_provider_metadata`, so a provider cannot be
        //              downgraded to `none` by the token itself);
        //   iss        against the discovered issuer, with a validated tenant
        //              substituted for Microsoft common;
        //   aud        must contain our `client_id`;
        //   exp        against the clock; iat has no additional age or clock check;
        //   nonce      against the value from the pending attempt.
        //
        // Nothing here is our own crypto, and that is the point.
        let verification_client = self.verification_client(&id_token.to_string())?;
        let claims = match id_token.claims(&verification_client.id_token_verifier(), &nonce) {
            Ok(claims) => claims,
            Err(ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey,
            )) => {
                // The token was signed by a key we have never seen. That is what
                // a key rotation looks like from here, so refetch the JWKS and
                // try once more — rate-limited, see `refresh_jwks`.
                self.refresh_jwks().await?;
                let client = self.verification_client(&id_token.to_string())?;
                id_token
                    .claims(&client.id_token_verifier(), &nonce)
                    .map_err(|err| AuthError::TokenVerification(err.to_string()))?
            }
            Err(err) => return Err(AuthError::TokenVerification(err.to_string())),
        };

        let identity = VerifiedIdentity {
            provider: self.kind,
            subject: claims.subject().as_str().to_owned(),
            avatar_url: claims
                .picture()
                .and_then(|picture| picture.get(None))
                .map(|picture| picture.as_str().to_owned()),
        };
        if let Some(subject) = revoke_subject {
            if self.kind != ProviderKind::Apple || subject != identity.subject {
                return Err(AuthError::TokenVerification(
                    "deletion subject mismatch".into(),
                ));
            }
            let (token, hint) = match token_response.refresh_token() {
                Some(token) => (token.secret().as_str(), "refresh_token"),
                None => (
                    token_response.access_token().secret().as_str(),
                    "access_token",
                ),
            };
            let secret = self.secret.client_secret()?;
            let response = self
                .http
                .post(self.revocation_url.clone())
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("client_secret", secret.secret().as_str()),
                    ("token", token),
                    ("token_type_hint", hint),
                ])
                .send()
                .await
                .map_err(|_| AuthError::Provider("Apple revocation could not be reached".into()))?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(AuthError::Provider(
                    "Apple revocation was not confirmed".into(),
                ));
            }
        }
        Ok(identity)
    }

    /// Refetches the JWKS, at most once per [`JWKS_MIN_REFETCH_INTERVAL`].
    ///
    /// The rate limit is claimed *before* the network call, so a burst of
    /// unknown-`kid` tokens produces one fetch rather than one per token.
    async fn refresh_jwks(&self) -> Result<(), AuthError> {
        let (jwks_uri, microsoft) = {
            let mut cached = self
                .metadata
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            if cached.jwks_fetched_at.elapsed() < JWKS_MIN_REFETCH_INTERVAL {
                let provider_name = self.kind.as_str();
                tracing::debug!(
                    provider = provider_name,
                    "unknown signing key, but a JWKS refetch happened recently — not refetching"
                );
                return Err(AuthError::TokenVerification(
                    "ID token signed by an unknown key".to_owned(),
                ));
            }

            cached.jwks_fetched_at = Instant::now();
            (
                cached.metadata.jwks_uri().clone(),
                cached.microsoft_jwks.is_some(),
            )
        };

        let provider_name = self.kind.as_str();
        tracing::info!(
            provider = provider_name,
            "refetching JWKS after an unknown signing key"
        );

        let (jwks, raw_keys) = if microsoft {
            let keys = fetch_json(&self.http, jwks_uri.as_str()).await?;
            let jwks = serde_json::from_value(keys.clone())
                .map_err(|err| AuthError::Provider(format!("invalid Microsoft JWKS: {err}")))?;
            (jwks, Some(keys))
        } else {
            let jwks = CoreJsonWebKeySet::fetch_async(&jwks_uri, &self.http)
                .await
                .map_err(|err| AuthError::Provider(format!("JWKS refetch failed: {err}")))?;
            (jwks, None)
        };

        let mut cached = self
            .metadata
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Clone and replace metadata because `set_jwks` consumes its input.
        cached.metadata = cached.metadata.clone().set_jwks(jwks);
        cached.microsoft_jwks = raw_keys;

        Ok(())
    }
}

/// Google OIDC and Microsoft OIDC, including the common authority.
pub(crate) struct StandardOidcProvider {
    core: OidcCore,
}

impl StandardOidcProvider {
    pub(crate) async fn discover(
        kind: ProviderKind,
        config: &crate::config::OidcProviderConfig,
        base_url: &Url,
    ) -> Result<Self, AuthError> {
        let core = OidcCore::discover(
            kind,
            &config.issuer,
            &config.client_id,
            Arc::new(StaticClientSecret::new(&config.client_secret)),
            super::callback_uri(base_url, kind)?,
            vec![Scope::new("profile".into())],
            Vec::new(),
        )
        .await?;

        Ok(Self { core })
    }
}

#[async_trait::async_trait]
impl crate::auth::AuthProvider for StandardOidcProvider {
    fn kind(&self) -> ProviderKind {
        self.core.kind
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
        // No form body: only Apple posts one, and only on first authorization.
        self.core.exchange(code, pkce_verifier, nonce).await
    }
}

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod tests;
