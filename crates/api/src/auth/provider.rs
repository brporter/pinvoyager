//! Provider abstraction.
//!
//! Three of the four providers are OIDC-conformant and share almost everything.
//! Facebook is not: it issues no ID token, so its subject comes from a Graph
//! API call instead. The trait exists so that difference lives in one
//! implementation rather than in every call site.

use std::collections::HashMap;

use async_trait::async_trait;
use url::Url;

use crate::auth::AuthError;
use crate::auth::providers::apple::AppleProvider;
use crate::auth::providers::facebook::FacebookProvider;
use crate::auth::providers::oidc::StandardOidcProvider;
use crate::db::repo::identities::Provider as DbProvider;

/// Which provider. Distinct from the database's `Provider` enum so the storage
/// representation and the protocol layer can move independently, with one
/// conversion point rather than a scattering of matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Google,
    Microsoft,
    Apple,
    Facebook,
}

impl ProviderKind {
    /// URL path segment, and the value stored in the database. Must match the
    /// migration's CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Google => "google",
            ProviderKind::Microsoft => "microsoft",
            ProviderKind::Apple => "apple",
            ProviderKind::Facebook => "facebook",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "google" => Some(ProviderKind::Google),
            "microsoft" => Some(ProviderKind::Microsoft),
            "apple" => Some(ProviderKind::Apple),
            "facebook" => Some(ProviderKind::Facebook),
            _ => None,
        }
    }
}

impl From<ProviderKind> for DbProvider {
    fn from(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Google => DbProvider::Google,
            ProviderKind::Microsoft => DbProvider::Microsoft,
            ProviderKind::Apple => DbProvider::Apple,
            ProviderKind::Facebook => DbProvider::Facebook,
        }
    }
}

/// What a provider tells us about someone, after verification.
///
/// Note the absence of an email field. This service does not store email
/// (D3), and not having it on the struct
/// means no handler can casually start.
#[derive(Debug, Clone)]
pub struct VerifiedIdentity {
    pub provider: ProviderKind,
    /// The provider's stable subject identifier. Opaque — never parsed.
    pub subject: String,
    pub avatar_url: Option<String>,
}

/// Everything needed to send the browser to the provider and to verify what
/// comes back.
pub struct AuthorizationRedirect {
    pub url: Url,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

#[async_trait]
pub trait AuthProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// Builds the authorization URL along with the state, nonce and PKCE
    /// verifier that the callback will need.
    fn authorize(&self) -> Result<AuthorizationRedirect, AuthError>;

    /// Exchanges the code and returns a **verified** identity.
    ///
    /// "Verified" means, for the OIDC providers: signature checked against the
    /// provider's JWKS, and `iss`, `aud`, `exp` and `nonce` all validated.
    /// Decoding a token is not verifying it, and an unverified token looks
    /// identical in a debugger.
    ///
    async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<VerifiedIdentity, AuthError>;
}

/// The configured providers.
///
/// Providers are individually optional. Obtaining four sets of OAuth
/// credentials is a serialized, human-gated process, and a service that cannot
/// start until all four exist cannot be worked on. An unconfigured provider's
/// endpoints return 404.
pub struct ProviderRegistry {
    providers: HashMap<ProviderKind, Box<dyn AuthProvider>>,
}

/// The order providers are offered in.
///
/// Fixed, because [`ProviderRegistry::configured`] feeds the sign-in screen and
/// a `HashMap`'s iteration order would reshuffle the buttons on every restart —
/// a UI that moves under a user who has learned where their provider is.
const DISPLAY_ORDER: [ProviderKind; 4] = [
    ProviderKind::Google,
    ProviderKind::Microsoft,
    ProviderKind::Apple,
    ProviderKind::Facebook,
];

impl ProviderRegistry {
    #[cfg(test)]
    pub(crate) fn for_test(kind: ProviderKind, provider: Box<dyn AuthProvider>) -> Self {
        Self {
            providers: HashMap::from([(kind, provider)]),
        }
    }

    /// Builds the registry, performing OIDC discovery once per configured
    /// provider.
    ///
    /// Discovery and JWKS are fetched here and cached for the process lifetime,
    /// not per sign-in — a discovery round trip on every authorization would
    /// add latency to every sign-in and hammer the provider. Key rotation is
    /// handled by refetching on an unknown `kid`, rate-limited so that bogus
    /// `kid`s cannot be used to drive traffic at the provider.
    ///
    /// A provider that fails discovery at startup must not take the whole
    /// service down: log it, disable that provider, and keep serving.
    pub async fn from_config(config: &crate::config::AuthConfig) -> Self {
        Self::from_config_with_apple_issuer(config, super::providers::apple::APPLE_ISSUER).await
    }

    async fn from_config_with_apple_issuer(
        config: &crate::config::AuthConfig,
        apple_issuer: &str,
    ) -> Self {
        let base_url = &config.base_url;
        let configured = &config.providers;

        // Discovery is three sequential HTTPS round trips if done one at a time,
        // and they are independent, so they are not done one at a time. This is
        // in the startup path, before the listener binds.
        let (google, microsoft, apple) = tokio::join!(
            async {
                match &configured.google {
                    Some(cfg) => Some(
                        StandardOidcProvider::discover(ProviderKind::Google, cfg, base_url)
                            .await
                            .map(boxed),
                    ),
                    None => None,
                }
            },
            async {
                match &configured.microsoft {
                    Some(cfg) => Some(
                        StandardOidcProvider::discover(ProviderKind::Microsoft, cfg, base_url)
                            .await
                            .map(boxed),
                    ),
                    None => None,
                }
            },
            async {
                match &configured.apple {
                    Some(cfg) => Some(
                        AppleProvider::discover(cfg, base_url, apple_issuer)
                            .await
                            .map(boxed),
                    ),
                    None => None,
                }
            },
        );

        // Facebook needs no discovery — it publishes none — so it is built
        // inline rather than joined.
        let facebook = configured
            .facebook
            .as_ref()
            .map(|cfg| FacebookProvider::new(cfg, base_url).map(boxed));

        let mut providers = HashMap::new();
        let outcomes = [
            (ProviderKind::Google, google),
            (ProviderKind::Microsoft, microsoft),
            (ProviderKind::Apple, apple),
            (ProviderKind::Facebook, facebook),
        ];

        for (kind, outcome) in outcomes {
            match outcome {
                Some(Ok(provider)) => {
                    tracing::info!(provider = kind.as_str(), "provider ready");
                    providers.insert(kind, provider);
                }
                // A provider whose discovery document is
                // unreachable — an outage, a proxy, a typo'd issuer — must not
                // stop the other three from serving. It is logged at warn and
                // its endpoints will 404 until the next restart, which is a
                // partial outage rather than a total one.
                //
                // Microsoft is the likeliest to land here: the `common` and
                // `consumers` issuers return `issuer` values that do not equal
                // the URL they were fetched from (`common` literally returns a
                // `{tenantid}` placeholder), and OIDC requires that comparison
                // to be exact. It needs a tenant-specific issuer URL.
                Some(Err(err)) => {
                    let provider_name = kind.as_str();
                    tracing::warn!(
                        provider = provider_name,
                        error = %err,
                        "provider disabled: it could not be initialized at startup"
                    );
                }
                None => {
                    tracing::debug!(provider = kind.as_str(), "provider not configured");
                }
            }
        }

        if providers.is_empty() {
            // Not an error. A deployment with no credentials yet must still
            // start; sign-in simply 404s until one is configured.
            tracing::warn!("no authentication providers are available — sign-in is disabled");
        }

        Self { providers }
    }

    /// A registry with nothing configured.
    ///
    /// Not only for tests: it is the honest representation of a deployment
    /// that has no provider credentials yet, and the service is required to
    /// start in that state rather than refuse to boot.
    pub fn empty() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn get(&self, kind: ProviderKind) -> Option<&dyn AuthProvider> {
        self.providers.get(&kind).map(Box::as_ref)
    }

    /// Which providers actually came up. Used by `/api/me` and by the sign-in
    /// screen so it only offers buttons that work.
    pub fn configured(&self) -> Vec<ProviderKind> {
        DISPLAY_ORDER
            .into_iter()
            .filter(|kind| self.providers.contains_key(kind))
            .collect()
    }
}

/// Erases a concrete provider to the trait object the registry stores.
fn boxed<P: AuthProvider + 'static>(provider: P) -> Box<dyn AuthProvider> {
    Box::new(provider)
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
