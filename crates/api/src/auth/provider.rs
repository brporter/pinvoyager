//! Provider abstraction.
//!
//! Google, Microsoft, and Apple use OIDC. Apple supplies a signed client secret
//! and a form callback; the shared trait keeps those differences in the provider.

use std::collections::HashMap;

use async_trait::async_trait;
use url::Url;

use crate::auth::AuthError;
use crate::auth::providers::apple::AppleProvider;
use crate::auth::providers::oidc::StandardOidcProvider;
use crate::db::repo::identities::Provider as DbProvider;

/// Provider identifier used by authentication and converted to the repository enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Google,
    Microsoft,
    Apple,
}

impl ProviderKind {
    /// Lowercase name used in URL paths and stored provider values.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Google => "google",
            ProviderKind::Microsoft => "microsoft",
            ProviderKind::Apple => "apple",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "google" => Some(ProviderKind::Google),
            "microsoft" => Some(ProviderKind::Microsoft),
            "apple" => Some(ProviderKind::Apple),
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
        }
    }
}

/// Verified provider identity. No provider credentials are persisted.
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

    /// Exchanges an authorization code and returns the verified identity.
    /// Verification checks the signature, issuer, audience, expiry, and nonce.
    /// Exchange or verification failures return an authentication error.
    async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<VerifiedIdentity, AuthError>;
}

/// The configured providers.
///
/// Providers are individually optional. The service starts with any subset of
/// credentials configured. An unconfigured provider's endpoints return 404.
pub struct ProviderRegistry {
    providers: HashMap<ProviderKind, Box<dyn AuthProvider>>,
}

/// Stable order used when reporting configured providers.
const DISPLAY_ORDER: [ProviderKind; 3] = [
    ProviderKind::Google,
    ProviderKind::Microsoft,
    ProviderKind::Apple,
];

impl ProviderRegistry {
    #[cfg(test)]
    pub(crate) fn for_test(kind: ProviderKind, provider: Box<dyn AuthProvider>) -> Self {
        Self {
            providers: HashMap::from([(kind, provider)]),
        }
    }

    /// Discovers configured providers at startup. Failed providers are logged and
    /// omitted; the service can start with none. Endpoints remain cached, while
    /// unknown signing keys can trigger rate-limited JWKS refreshes.
    pub async fn from_config(config: &crate::config::AuthConfig) -> Self {
        Self::from_config_with_apple_issuer(config, super::providers::apple::APPLE_ISSUER).await
    }

    async fn from_config_with_apple_issuer(
        config: &crate::config::AuthConfig,
        apple_issuer: &str,
    ) -> Self {
        let base_url = &config.base_url;
        let configured = &config.providers;

        // Discover configured providers concurrently before binding the HTTP listener.
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

        let mut providers = HashMap::new();
        let outcomes = [
            (ProviderKind::Google, google),
            (ProviderKind::Microsoft, microsoft),
            (ProviderKind::Apple, apple),
        ];

        for (kind, outcome) in outcomes {
            match outcome {
                Some(Ok(provider)) => {
                    tracing::info!(provider = kind.as_str(), "provider ready");
                    providers.insert(kind, provider);
                }
                // Log discovery failure and omit this provider until the next startup.
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

    /// Returns an empty provider registry for deployments without credentials and tests.
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
