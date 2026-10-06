//! Facebook Login — OAuth2, **not** OIDC.
//!
//! There is no ID token here, so there is nothing to verify a signature on and
//! no `nonce` to bind. What identifies the user is the app-scoped ID returned by
//! the Graph API, which is stable for this app and this person and is useless to
//! any other app. That is the subject we store.
//!
//! The security of this flow therefore rests on three things and no fourth:
//!
//! - `state`, checked by the caller against the pending attempt (CSRF);
//! - PKCE, which binds the code to the browser that started the flow;
//! - the fact that the code is redeemed over TLS by us, using our app secret,
//!   and the `/me` response comes back on that same connection.
//!
//! Trying to make this look like OIDC — pretending the access token is an
//! assertion, or decoding anything out of it — would be the mistake. It is an
//! opaque bearer token, we use it for exactly one request, and then it is gone
//! (D1).

use openidconnect::{CsrfToken, Nonce, PkceCodeChallenge};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

use crate::auth::AuthError;
use crate::auth::provider::{AuthorizationRedirect, ProviderKind, VerifiedIdentity};
use crate::config::FacebookProviderConfig;

/// Graph API version, pinned deliberately.
///
/// An unversioned Graph call is served by the *oldest* version still available,
/// which changes underneath the service without warning. A pinned version is
/// supported for about two years from release, so this constant is a scheduled
/// maintenance item, not a set-and-forget.
const GRAPH_VERSION: &str = "v23.0";

/// Only `public_profile`, which is what `id`, `name` and `picture` need.
///
/// Notably not `email`: this service does not store an email address, so asking
/// for one would mean showing the user a consent screen for data we intend to
/// throw away.
const SCOPE: &str = "public_profile";

/// Exactly the fields used below. Graph returns whatever is asked for, so the
/// list is the whole of the data-minimization story for this provider.
const ME_FIELDS: &str = "id,name,picture.type(large)";

pub(crate) struct FacebookProvider {
    client_id: String,
    client_secret: SecretString,
    redirect_uri: String,
    graph_api_root: Url,
    http: reqwest::Client,
}

impl FacebookProvider {
    /// No discovery: Facebook publishes no discovery document, and its endpoints
    /// are version-pinned constants. Nothing here does I/O, so unlike the OIDC
    /// three this provider cannot fail to come up for a reason outside our
    /// control.
    pub(crate) fn new(config: &FacebookProviderConfig, base_url: &Url) -> Result<Self, AuthError> {
        let graph_api_root = Url::parse(&format!("https://graph.facebook.com/{GRAPH_VERSION}/"))
            .map_err(|err| AuthError::Configuration(format!("bad Graph API URL: {err}")))?;
        Self::build(config, base_url, graph_api_root)
    }

    fn build(
        config: &FacebookProviderConfig,
        base_url: &Url,
        graph_api_root: Url,
    ) -> Result<Self, AuthError> {
        let redirect_uri = super::callback_uri(base_url, ProviderKind::Facebook)?;

        // The same client the OIDC providers use, for the same reasons: a
        // bounded timeout, and no following of redirects on requests that carry
        // the app secret or a bearer token.
        let http = super::provider_http_client()?;

        Ok(Self {
            client_id: config.client_id.clone(),
            // `SecretString` is not `Clone` by design, so this copies the bytes
            // into a second redacting wrapper rather than out of one.
            client_secret: SecretString::from(config.client_secret.expose_secret()),
            redirect_uri: redirect_uri.as_str().to_owned(),
            graph_api_root,
            http,
        })
    }

    fn graph_url(&self, path: &str) -> Result<Url, AuthError> {
        self.graph_api_root
            .join(path)
            .map_err(|err| AuthError::Configuration(format!("bad Graph API URL: {err}")))
    }

    fn authorize_at(&self, endpoint: &str) -> Result<AuthorizationRedirect, AuthError> {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let state = CsrfToken::new_random();
        // Facebook has no ID token and so nothing to carry a `nonce`. One is
        // still generated because the pending attempt has a column for it and a
        // uniform record is easier to reason about than a conditional one; it is
        // simply never checked, because there is nothing to check it against.
        let nonce = Nonce::new_random();

        let url = facebook_authorization_url(
            endpoint,
            &[
                ("client_id", self.client_id.as_str()),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("state", state.secret().as_str()),
                ("response_type", "code"),
                ("scope", SCOPE),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", challenge.method().as_str()),
            ],
        )?;

        Ok(AuthorizationRedirect {
            url,
            state: state.secret().to_owned(),
            nonce: nonce.secret().to_owned(),
            pkce_verifier: verifier.secret().to_owned(),
        })
    }

    async fn fetch_profile(&self, access_token: &str) -> Result<MeResponse, AuthError> {
        let mut url = self.graph_url("me")?;
        url.query_pairs_mut().append_pair("fields", ME_FIELDS);

        // The token goes in the header rather than in the query string, where it
        // would end up in access logs and `Referer` headers on the way.
        //
        // Not sent: `appsecret_proof`. Computing it needs HMAC-SHA256 over the
        // access token, and this crate has no HMAC dependency. The app must
        // therefore be left with "Require app secret proof for server API calls"
        // off, or this call fails.
        let response = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|err| AuthError::Provider(format!("Graph /me failed: {err}")))?
            .error_for_status()
            .map_err(|err| AuthError::Provider(format!("Graph /me was rejected: {err}")))?;

        response
            .json()
            .await
            .map_err(|err| AuthError::Provider(format!("unreadable Graph /me response: {err}")))
    }
}

#[async_trait::async_trait]
impl crate::auth::AuthProvider for FacebookProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Facebook
    }

    fn authorize(&self) -> Result<AuthorizationRedirect, AuthError> {
        self.authorize_at(&format!(
            "https://www.facebook.com/{GRAPH_VERSION}/dialog/oauth"
        ))
    }

    async fn exchange(
        &self,
        code: &str,
        pkce_verifier: &str,
        _nonce: &str,
        _form_body: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        // Encoded explicitly rather than through `reqwest`'s `form()` helper,
        // which needs the optional `form` feature that this crate does not
        // enable. `url`'s serializer produces the same bytes.
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.client_id)
            .append_pair("client_secret", self.client_secret.expose_secret())
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("code", code)
            .append_pair("code_verifier", pkce_verifier)
            .finish();

        let response = self
            .http
            .post(self.graph_url("oauth/access_token")?)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|err| AuthError::Provider(format!("code exchange failed: {err}")))?;

        // Facebook answers a failed exchange with 400 and a JSON error body. The
        // status is checked before parsing so a rejection does not present as a
        // deserialization error.
        let response = response
            .error_for_status()
            .map_err(|err| AuthError::Provider(format!("code exchange was rejected: {err}")))?;

        let token: TokenResponse = response
            .json()
            .await
            .map_err(|err| AuthError::Provider(format!("unreadable token response: {err}")))?;

        let profile = self.fetch_profile(&token.access_token).await;

        // D1, made explicit: the access token dies here whether or not the Graph
        // call worked. It is never returned, logged, or written down.
        drop(token);

        let profile = profile?;

        Ok(VerifiedIdentity {
            provider: ProviderKind::Facebook,
            // App-scoped: stable for this app and this person, and meaningless
            // to any other app. This is the subject, and it is never parsed.
            subject: profile.id,
            display_name: profile.name.filter(|name| !name.trim().is_empty()),
            avatar_url: profile
                .picture
                .and_then(|picture| picture.data)
                .and_then(|data| {
                    // The silhouette is Facebook's placeholder for "no avatar".
                    // Storing it would give every avatar-less user the same
                    // stock image instead of our own initials fallback.
                    if data.is_silhouette.unwrap_or(false) {
                        None
                    } else {
                        data.url
                    }
                }),
        })
    }
}

/// Only the one field that is used. `expires_in` and `token_type` are ignored
/// rather than captured: nothing is allowed to want them (D1).
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct MeResponse {
    id: String,
    name: Option<String>,
    picture: Option<Picture>,
}

#[derive(Deserialize)]
struct Picture {
    data: Option<PictureData>,
}

#[derive(Deserialize)]
struct PictureData {
    url: Option<String>,
    is_silhouette: Option<bool>,
}

fn facebook_authorization_url(base_url: &str, params: &[(&str, &str)]) -> Result<Url, AuthError> {
    Url::parse_with_params(base_url, params).map_err(|err| {
        AuthError::Configuration(format!("could not build the Facebook authorize URL: {err}"))
    })
}

#[cfg(test)]
#[path = "facebook_tests.rs"]
mod tests;
