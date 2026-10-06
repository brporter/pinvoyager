//! The four provider implementations behind [`AuthProvider`].
//!
//! [`AuthProvider`]: crate::auth::AuthProvider
//!
//! Three of them (Google, Microsoft, Apple) are OIDC and share [`oidc::OidcCore`];
//! Facebook is not OIDC at all and has no ID token to verify, so it talks to the
//! Graph API instead. Apple is OIDC but deviates in three ways that are large
//! enough to justify its own module — see [`apple`].
//!
//! ## What "verified" means, and where it happens
//!
//! For the OIDC three, every check required by D1 is performed by
//! `openidconnect` inside `IdToken::claims()`:
//!
//! - **signature** against the JWKS fetched at discovery, restricted to the
//!   algorithms the provider's own metadata advertises;
//! - **`iss`** against the issuer in the discovery document;
//! - **`aud`** against our `client_id`;
//! - **`exp`** (and `iat`) against the clock;
//! - **`nonce`** against the value we stored in the pending attempt.
//!
//! The risk in this phase was never implementing those checks — it was
//! *configuring* them wrongly, since a misconfigured verifier still returns
//! `Ok` and an unverified token looks identical in a debugger. The call sites in
//! [`oidc`] name each check where it is enabled.
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
pub(crate) mod facebook;
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

/// The redirect URI we send to the provider and repeat at the code exchange.
///
/// It must match what is registered in the provider's console *exactly* —
/// every provider compares it byte for byte and rejects a mismatch with an
/// error that does not say so. Building it from one place means the `/start`
/// redirect and the token request cannot drift apart, which is the failure this
/// helper exists to prevent.
pub(crate) fn callback_uri(base_url: &Url, kind: ProviderKind) -> Result<RedirectUrl, AuthError> {
    let path = format!("/auth/{}/callback", kind.as_str());
    let url = base_url.join(&path).map_err(|err| {
        AuthError::Configuration(format!("auth.base_url is not a usable base URL: {err}"))
    })?;
    Ok(RedirectUrl::from_url(url))
}

/// The HTTP client every provider call goes through — discovery, JWKS, token
/// endpoints and Facebook's Graph API alike.
///
/// `redirect::Policy::none()` is not a detail: `oauth2` documents that following
/// redirects on these requests turns the client into an SSRF gadget, because a
/// provider (or anything that can answer for it) could redirect a request that
/// carries our client secret somewhere else entirely.
///
/// One function rather than one per provider because there is exactly one
/// `reqwest` in the build: `crates/api/Cargo.toml` pins it to 0.12 so that
/// `&reqwest::Client` is the type `oauth2`'s `AsyncHttpClient` is implemented
/// for. If that pin ever moves, `openidconnect::reqwest::Client` and
/// `reqwest::Client` become two different types and this splits in two.
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
