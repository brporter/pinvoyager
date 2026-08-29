//! The OIDC machinery shared by Google, Microsoft and Apple.
//!
//! Discovery happens **once**, in [`OidcCore::discover`], and the resulting
//! metadata — endpoints, supported algorithms, and the JWKS — is kept for the
//! process lifetime. A discovery round trip per sign-in would add a hop to every
//! authorization and put avoidable load on the provider for a document that
//! changes about once a year.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreProviderMetadata,
};
use openidconnect::{
    AuthType, AuthorizationCode, ClaimsVerificationError, ClientId, ClientSecret, CsrfToken,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, SignatureVerificationError, TokenResponse,
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

/// Floor on how often a provider's JWKS may be refetched.
///
/// The refetch is triggered by an ID token whose `kid` is not in the cached key
/// set, which is exactly what a legitimate key rotation looks like — and also
/// exactly what an attacker posting garbage tokens looks like. Without a floor,
/// unauthenticated requests to our callback would turn into unbounded requests
/// to the provider's JWKS endpoint, which is a way to get rate-limited by (or
/// blamed by) the provider.
const JWKS_MIN_REFETCH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Where the `client_secret` presented at the token endpoint comes from.
///
/// Three of the providers have a static string. Apple's is a JWT that this
/// service mints and that expires, so it cannot be captured once at startup —
/// hence the indirection rather than a `ClientSecret` field.
pub(crate) trait ClientSecretSource: Send + Sync {
    fn client_secret(&self) -> Result<ClientSecret, AuthError>;
}

/// A secret that came out of configuration and never changes.
///
/// Held as `oauth2`'s `ClientSecret` rather than `SecretString` because that is
/// what the token request wants and because `SecretString` is deliberately not
/// `Clone`. Both redact themselves in `Debug`, so nothing is given up.
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
    scopes: Vec<Scope>,
    /// Authorization-request parameters the OIDC spec does not model. Apple's
    /// `response_mode=form_post` is the only user today.
    extra_authorize_params: Vec<(&'static str, &'static str)>,
    http: reqwest::Client,
    metadata: RwLock<CachedMetadata>,
}

/// Turns a configured issuer into the form discovery will accept.
///
/// `Url` normalizes `https://accounts.google.com` into
/// `https://accounts.google.com/`, and OIDC requires the `issuer` in the
/// discovery document to equal the identifier it was fetched for *exactly* —
/// `openidconnect` compares the raw strings. Google returns the form without the
/// slash, so passing the normalized `Url` straight through fails discovery for
/// the most ordinary configuration imaginable, with an error about issuer
/// mismatch that reads like a security problem rather than a typo.
///
/// Trimming is safe: an issuer identifier is defined as having no trailing
/// slash.
fn issuer_url(issuer: &Url) -> Result<IssuerUrl, AuthError> {
    IssuerUrl::new(issuer.as_str().trim_end_matches('/').to_owned())
        .map_err(|err| AuthError::Configuration(format!("invalid issuer URL: {err}")))
}

impl OidcCore {
    /// Fetches the discovery document and the JWKS, once.
    pub(crate) async fn discover(
        kind: ProviderKind,
        issuer: &Url,
        client_id: &str,
        secret: Arc<dyn ClientSecretSource>,
        redirect_uri: RedirectUrl,
        scopes: Vec<Scope>,
        extra_authorize_params: Vec<(&'static str, &'static str)>,
    ) -> Result<Self, AuthError> {
        let issuer = issuer_url(issuer)?;
        let http = provider_http_client()?;

        // This fetches `{issuer}/.well-known/openid-configuration` *and* the
        // JWKS it points at, and rejects a document whose `issuer` claim is not
        // the issuer we asked for.
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|err| AuthError::Configuration(format!("OIDC discovery failed: {err}")))?;

        Ok(Self {
            kind,
            client_id: ClientId::new(client_id.to_owned()),
            secret,
            redirect_uri,
            scopes,
            extra_authorize_params,
            http,
            metadata: RwLock::new(CachedMetadata {
                metadata,
                jwks_fetched_at: Instant::now(),
            }),
        })
    }

    /// Builds a client from the cached metadata.
    ///
    /// Cheap and local — no I/O. It is done per request rather than once at
    /// startup because both inputs can change underneath it: Apple's client
    /// secret expires, and the JWKS is replaced when a provider rotates keys.
    /// Rebuilding is a few clones; caching it would mean invalidation logic for
    /// no measurable gain next to a network round trip.
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
        // refresh token. Neither is read, returned, or stored — both are dropped
        // with this binding at the end of the function.
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
        //   iss        against the discovered issuer;
        //   aud        must contain our `client_id`;
        //   exp/iat    against the clock;
        //   nonce      against the value from the pending attempt.
        //
        // Nothing here is our own crypto, and that is the point.
        let claims = match id_token.claims(&client.id_token_verifier(), &nonce) {
            Ok(claims) => claims,
            Err(ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey,
            )) => {
                // The token was signed by a key we have never seen. That is what
                // a key rotation looks like from here, so refetch the JWKS and
                // try once more — rate-limited, see `refresh_jwks`.
                self.refresh_jwks().await?;
                let client = self.client(Some(self.secret.client_secret()?));
                id_token
                    .claims(&client.id_token_verifier(), &nonce)
                    .map_err(|err| AuthError::TokenVerification(err.to_string()))?
            }
            Err(err) => return Err(AuthError::TokenVerification(err.to_string())),
        };

        Ok(VerifiedIdentity {
            provider: self.kind,
            subject: claims.subject().as_str().to_owned(),
            // Absent for Apple, which sends the name in the form body on first
            // authorization only, and often absent for Microsoft. Optional
            // everywhere rather than defaulted to something invented.
            display_name: claims
                .name()
                .and_then(|name| name.get(None))
                .map(|name| name.as_str().to_owned()),
            avatar_url: claims
                .picture()
                .and_then(|picture| picture.get(None))
                .map(|picture| picture.as_str().to_owned()),
        })
    }

    /// Refetches the JWKS, at most once per [`JWKS_MIN_REFETCH_INTERVAL`].
    ///
    /// The rate limit is claimed *before* the network call, so a burst of
    /// unknown-`kid` tokens produces one fetch rather than one per token.
    async fn refresh_jwks(&self) -> Result<(), AuthError> {
        let jwks_uri = {
            let mut cached = self
                .metadata
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            if cached.jwks_fetched_at.elapsed() < JWKS_MIN_REFETCH_INTERVAL {
                tracing::debug!(
                    provider = self.kind.as_str(),
                    "unknown signing key, but a JWKS refetch happened recently — not refetching"
                );
                return Err(AuthError::TokenVerification(
                    "ID token signed by an unknown key".to_owned(),
                ));
            }

            cached.jwks_fetched_at = Instant::now();
            cached.metadata.jwks_uri().clone()
        };

        tracing::info!(
            provider = self.kind.as_str(),
            "refetching JWKS after an unknown signing key"
        );

        let jwks = CoreJsonWebKeySet::fetch_async(&jwks_uri, &self.http)
            .await
            .map_err(|err| AuthError::Provider(format!("JWKS refetch failed: {err}")))?;

        let mut cached = self
            .metadata
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `set_jwks` consumes the metadata, so this is clone-modify-replace.
        // It happens at most once per rotation, not per request.
        cached.metadata = cached.metadata.clone().set_jwks(jwks);

        Ok(())
    }
}

/// Google and Microsoft: OIDC with nothing bent out of shape.
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
            // `openid` is added by the library. `profile` is what supplies a
            // display name and (from Google) an avatar. `email` is deliberately
            // absent: this service does not store an email address, and asking a
            // provider for data we must immediately discard is a privacy cost
            // with no product behind it.
            vec![Scope::new("profile".to_owned())],
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
        _form_body: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        // No form body: only Apple posts one, and only on first authorization.
        self.core.exchange(code, pkce_verifier, nonce).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
