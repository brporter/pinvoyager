//! The three provider implementations behind [`AuthProvider`].
//!
//! [`AuthProvider`]: crate::auth::AuthProvider
//!
//! Google, Microsoft, and Apple share [`oidc::OidcCore`]. Apple mints signed
//! client secrets and uses a form-POST callback — see [`apple`].
//!
//! ## What "verified" means, and where it happens
//!
//! For the OIDC three, every check required by D1 is performed by
//! `openidconnect` inside `IdToken::claims()`:
//!
//! - **signature** against the JWKS fetched at discovery, restricted to the
//!   algorithms the provider's own metadata advertises;
//! - **`iss`** against the discovered issuer (Microsoft common substitutes the validated tenant);
//! - **`aud`** against our `client_id`;
//! - **`exp`** (and `iat`) against the clock;
//! - **`nonce`** against the value we stored in the pending attempt.
//!
//! The call sites in [`oidc`] identify where verification is configured.
//!
//! ## D1
//!
//! No access token or refresh token leaves these modules. The token response is
//! consumed for its ID token, the ID token is verified for its subject, and both
//! are dropped when `exchange` returns. [`VerifiedIdentity`] has nowhere to put
//! a credential, which is the structural half of the guarantee.
//!
//! [`VerifiedIdentity`]: crate::auth::VerifiedIdentity

pub(crate) mod apple;
pub(crate) mod oidc;

use std::time::Duration;

use openidconnect::RedirectUrl;
use url::Url;

use crate::auth::AuthError;
use crate::auth::provider::ProviderKind;

/// Ceiling on any single call to a provider. A provider that has gone slow must
/// surface as a failed sign-in, not as a request occupying a connection until
/// the global request timeout fires.
const PROVIDER_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the provider callback URI from the public base URL and provider name.
/// Returns a configuration error if the base cannot be joined with the callback path.
pub(crate) fn callback_uri(base_url: &Url, kind: ProviderKind) -> Result<RedirectUrl, AuthError> {
    let path = format!("/auth/{}/callback", kind.as_str());
    let url = base_url.join(&path).map_err(|err| {
        AuthError::Configuration(format!("auth.base_url is not a usable base URL: {err}"))
    })?;
    Ok(RedirectUrl::from_url(url))
}

/// Builds a provider HTTP client with a ten-second timeout and redirects disabled
/// to prevent forwarding credentials to a redirected destination. Client setup
/// failures return a configuration error.
pub(crate) fn provider_http_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROVIDER_HTTP_TIMEOUT)
        .build()
        .map_err(|err| AuthError::Configuration(format!("could not build an HTTP client: {err}")))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
