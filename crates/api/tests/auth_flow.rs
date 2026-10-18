//! End-to-end authentication callback behavior through the real router.

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::config::Config;
use pinvoyager_api::http;
use pinvoyager_api::photos::worker::VariantQueue;
use pinvoyager_api::state::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use url::Url;
use uuid::Uuid;

use support::MockOidcProvider;

struct ScratchRoot(PathBuf);

impl ScratchRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pv-auth-flow-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&path).expect("create auth test storage root");
        Self(path)
    }
}

impl Drop for ScratchRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn app(pool: PgPool, issuer: &Url, root: &ScratchRoot) -> Router {
    app_with_providers(pool, issuer, root, false).await
}

async fn app_with_microsoft(pool: PgPool, issuer: &Url, root: &ScratchRoot) -> Router {
    app_with_providers(pool, issuer, root, true).await
}

async fn app_with_providers(
    pool: PgPool,
    issuer: &Url,
    root: &ScratchRoot,
    microsoft_enabled: bool,
) -> Router {
    let mut config: Config = serde_json::from_value(json!({
        "server": { "bind": "127.0.0.1:0", "request_timeout_secs": 10 },
        "database": {
            "url": "postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager",
            "max_connections": 4,
            "min_connections": 1,
            "acquire_timeout_secs": 5
        },
        "telemetry": { "log_level": "warn", "json": false },
        "auth": {
            "base_url": "http://127.0.0.1:8080",
            "providers": {
                "google": {
                    "client_id": support::CLIENT_ID,
                    "client_secret": support::CLIENT_SECRET,
                    "issuer": issuer.as_str().trim_end_matches('/')
                },
                "microsoft": if microsoft_enabled {
                    json!({
                        "client_id": support::CLIENT_ID,
                        "client_secret": support::CLIENT_SECRET,
                        "issuer": issuer.as_str().trim_end_matches('/')
                    })
                } else {
                    Value::Null
                }
            }
        },
        "photos": { "storage_root": root.0 }
    }))
    .expect("valid test config");

    config.auth.dev_login = true;
    let providers = ProviderRegistry::from_config(&config.auth).await;
    http::router(AppState::new(
        pool,
        Arc::new(config),
        providers,
        VariantQueue::detached(),
    ))
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).expect("valid request"))
        .await
        .expect("router is infallible")
}

async fn send_form(
    app: &Router,
    uri: &str,
    cookie: Option<&str>,
    body: &str,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    app.clone()
        .oneshot(
            request
                .body(Body::from(body.to_owned()))
                .expect("valid form request"),
        )
        .await
        .expect("router is infallible")
}

struct StartedFlow {
    state: String,
    nonce: String,
    binding_cookie: String,
}

async fn start_flow(app: &Router, issuer: &Url, redirect_to: &str) -> StartedFlow {
    start_flow_for(app, issuer, "google", redirect_to).await
}

async fn start_flow_for(
    app: &Router,
    issuer: &Url,
    provider: &str,
    redirect_to: &str,
) -> StartedFlow {
    let redirect = url::form_urlencoded::byte_serialize(redirect_to.as_bytes()).collect::<String>();
    let started = send(
        app,
        "GET",
        &format!("/auth/{provider}/start?redirect_to={redirect}"),
        None,
    )
    .await;
    assert_eq!(started.status(), StatusCode::FOUND);

    let authorization_url = Url::parse(
        started.headers()[header::LOCATION]
            .to_str()
            .expect("location is valid"),
    )
    .expect("provider authorization URL");
    assert_eq!(
        authorization_url.origin().ascii_serialization(),
        issuer.origin().ascii_serialization()
    );
    let parameter = |key: &str| {
        authorization_url
            .query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| panic!("authorization carries {key}"))
    };
    let binding_cookie = started.headers()[header::SET_COOKIE]
        .to_str()
        .expect("binding cookie is valid")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    assert!(binding_cookie.starts_with("pv_auth_binding="));

    StartedFlow {
        state: parameter("state"),
        nonce: parameter("nonce"),
        binding_cookie,
    }
}

fn callback_uri(state: &str, code: Option<&str>, error: Option<&str>) -> String {
    callback_uri_for("google", state, code, error)
}

fn callback_uri_for(
    provider: &str,
    state: &str,
    code: Option<&str>,
    error: Option<&str>,
) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("state", state);
    if let Some(code) = code {
        query.append_pair("code", code);
    }
    if let Some(error) = error {
        query.append_pair("error", error);
    }
    format!("/auth/{provider}/callback?{}", query.finish())
}

async fn complete_sign_in(app: &Router, provider: &str, flow: &StartedFlow, code: &str) -> String {
    let completed = send(
        app,
        "GET",
        &callback_uri_for(provider, &flow.state, Some(code), None),
        Some(&flow.binding_cookie),
    )
    .await;
    assert_eq!(completed.status(), StatusCode::SEE_OTHER);
    completed
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("pv_session="))
        .expect("sign-in issues a session")
        .split(';')
        .next()
        .expect("session cookie pair")
        .to_owned()
}

#[sqlx::test]
async fn oidc_start_callback_and_current_user_complete_a_sign_in(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app(pool, provider.issuer(), &root).await;

    let flow = start_flow(&app, provider.issuer(), "/settings").await;
    let code = provider
        .token()
        .subject("end-to-end-subject")
        .nonce(&flow.nonce)
        .issue();
    let completed = send(
        &app,
        "GET",
        &callback_uri(&flow.state, Some(&code), None),
        Some(&flow.binding_cookie),
    )
    .await;
    assert_eq!(completed.status(), StatusCode::SEE_OTHER);
    assert_eq!(completed.headers()[header::LOCATION], "/settings");

    let cookies = completed.headers().get_all(header::SET_COOKIE);
    let session_cookie = cookies
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("pv_session="))
        .expect("successful callback issues a session")
        .split(';')
        .next()
        .expect("session cookie pair")
        .to_owned();
    assert!(
        cookies
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| { value.starts_with("pv_auth_binding=") && value.contains("Max-Age=0") })
    );

    let current_user = send(&app, "GET", "/api/me", Some(&session_cookie)).await;
    assert_eq!(current_user.status(), StatusCode::OK);
    let body = axum::body::to_bytes(current_user.into_body(), 16 * 1024)
        .await
        .expect("small current-user response");
    let body: Value = serde_json::from_slice(&body).expect("valid current-user JSON");
    assert!(
        body["user"]["display_name"].is_null(),
        "provider labels are not claimed handles"
    );
    assert_eq!(body["identities"][0]["provider"], "google");
    assert_eq!(
        body["identities"].as_array().expect("identity list").len(),
        1
    );
}

#[sqlx::test]
async fn logout_revokes_the_session_and_clears_the_cookie_even_if_already_signed_out(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app(pool, provider.issuer(), &root).await;

    let flow = start_flow(&app, provider.issuer(), "/").await;
    let code = provider
        .token()
        .subject("logout-subject")
        .nonce(&flow.nonce)
        .issue();
    let session_cookie = complete_sign_in(&app, "google", &flow, &code).await;

    let response = send(&app, "POST", "/auth/logout", Some(&session_cookie)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|value| {
                value.to_str().is_ok_and(|value| {
                    value.starts_with("pv_session=") && value.contains("Max-Age=0")
                })
            })
    );

    let revoked = send(&app, "GET", "/api/me", Some(&session_cookie)).await;
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

    let already_signed_out = send(&app, "POST", "/auth/logout", None).await;
    assert_eq!(already_signed_out.status(), StatusCode::NO_CONTENT);
    assert!(
        already_signed_out
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .all(|value| !value
                .to_str()
                .is_ok_and(|value| value.starts_with("pv_session=")))
    );
}

#[sqlx::test]
async fn callback_rejections_are_generic_and_clear_the_binding_cookie(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app(pool, provider.issuer(), &root).await;

    for (uri, expected) in [
        (
            "/auth/google/callback?code=missing-state",
            "/auth/callback?status=failed&provider=google",
        ),
        (
            "/auth/google/callback?code=unknown&state=unknown-state",
            "/auth/callback?status=failed&provider=google",
        ),
    ] {
        let response = send(&app, "GET", uri, None).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(
            response.headers()[header::LOCATION]
                .to_str()
                .expect("location is valid")
                .starts_with(expected)
        );
    }

    let malformed_form = send_form(&app, "/auth/google/callback", None, "code=missing-state").await;
    assert_eq!(malformed_form.status(), StatusCode::SEE_OTHER);
    assert!(
        malformed_form.headers()[header::LOCATION]
            .to_str()
            .expect("location is valid")
            .starts_with("/auth/callback?status=failed&provider=google")
    );
    assert_eq!(
        send_form(&app, "/auth/no-such-provider/callback", None, "state=x")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let missing_cookie = start_flow(&app, provider.issuer(), "/").await;
    let response = send(
        &app,
        "GET",
        &callback_uri(&missing_cookie.state, Some("unused"), None),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture")
            .contains("status=failed")
    );
    assert!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|cookie| {
                cookie
                    .to_str()
                    .unwrap_or_default()
                    .contains("pv_auth_binding=")
                    && cookie.to_str().unwrap_or_default().contains("Max-Age=0")
            })
    );

    let mismatched_cookie = start_flow(&app, provider.issuer(), "/").await;
    let response = send(
        &app,
        "GET",
        &callback_uri(&mismatched_cookie.state, Some("unused"), None),
        Some("pv_auth_binding=wrong"),
    )
    .await;
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture")
            .contains("status=failed")
    );

    let wrong_provider = start_flow(&app, provider.issuer(), "/").await;
    let response = send(
        &app,
        "GET",
        &callback_uri(&wrong_provider.state, Some("unused"), None)
            .replace("/auth/google/", "/auth/apple/"),
        Some(&wrong_provider.binding_cookie),
    )
    .await;
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture")
            .contains("status=failed")
    );

    let canceled = start_flow(&app, provider.issuer(), "/").await;
    let response = send(
        &app,
        "GET",
        &callback_uri(&canceled.state, None, Some("access_denied")),
        Some(&canceled.binding_cookie),
    )
    .await;
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("location is valid")
            .starts_with("/auth/callback?status=canceled&provider=google")
    );

    let provider_error = start_flow(&app, provider.issuer(), "/").await;
    let response = send(
        &app,
        "GET",
        &callback_uri(&provider_error.state, None, Some("temporarily_unavailable")),
        Some(&provider_error.binding_cookie),
    )
    .await;
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture")
            .contains("status=failed")
    );

    let missing_code = start_flow(&app, provider.issuer(), "/").await;
    let response = send(
        &app,
        "GET",
        &callback_uri(&missing_code.state, None, None),
        Some(&missing_code.binding_cookie),
    )
    .await;
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture")
            .contains("status=failed")
    );

    let invalid_token = start_flow(&app, provider.issuer(), "/").await;
    let code = provider
        .token()
        .nonce(&invalid_token.nonce)
        .audience("another-client")
        .issue();
    let response = send(
        &app,
        "GET",
        &callback_uri(&invalid_token.state, Some(&code), None),
        Some(&invalid_token.binding_cookie),
    )
    .await;
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture")
            .contains("status=failed")
    );
}

#[sqlx::test]
async fn form_callback_accepts_the_oidc_code_and_state_fields(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app(pool, provider.issuer(), &root).await;
    let flow = start_flow(&app, provider.issuer(), "/discover").await;
    let code = provider
        .token()
        .nonce(&flow.nonce)
        .subject("form-callback-subject")
        .issue();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("state", &flow.state)
        .finish();

    let response = send_form(
        &app,
        "/auth/google/callback",
        Some(&flow.binding_cookie),
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/discover");
}

#[sqlx::test]
async fn linking_a_new_provider_attaches_it_to_the_current_account_without_reissuing_a_session(
    pool: PgPool,
) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app_with_microsoft(pool, provider.issuer(), &root).await;

    let sign_in = start_flow(&app, provider.issuer(), "/settings").await;
    let code = provider
        .token()
        .subject("link-owner")
        .nonce(&sign_in.nonce)
        .issue();
    let session_cookie = complete_sign_in(&app, "google", &sign_in, &code).await;

    let started = send(
        &app,
        "POST",
        "/auth/link/microsoft?redirect_to=%2Fsettings",
        Some(&session_cookie),
    )
    .await;
    assert_eq!(started.status(), StatusCode::SEE_OTHER);
    let authorization = Url::parse(
        started.headers()[header::LOCATION]
            .to_str()
            .expect("provider redirect is valid"),
    )
    .expect("provider URL parses");
    let query = |key: &str| {
        authorization
            .query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
            .expect("authorization query field exists")
    };
    let state = query("state");
    let nonce = query("nonce");
    let binding_cookie = started.headers()[header::SET_COOKIE]
        .to_str()
        .expect("binding cookie is valid")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();

    let code = provider
        .token()
        .subject("linked-provider-subject")
        .nonce(&nonce)
        .issue();
    let linked = send(
        &app,
        "GET",
        &callback_uri_for("microsoft", &state, Some(&code), None),
        Some(&binding_cookie),
    )
    .await;
    assert_eq!(linked.status(), StatusCode::SEE_OTHER);
    assert_eq!(linked.headers()[header::LOCATION], "/settings");
    assert!(
        !linked
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.starts_with("pv_session="))
    );

    let current = send(&app, "GET", "/api/me", Some(&session_cookie)).await;
    assert_eq!(current.status(), StatusCode::OK);
    let body = axum::body::to_bytes(current.into_body(), 16 * 1024)
        .await
        .expect("small current-user response");
    let body: Value = serde_json::from_slice(&body).expect("valid current-user JSON");
    let identities = body["identities"].as_array().expect("identity list");
    assert_eq!(identities.len(), 2);
    assert!(
        identities
            .iter()
            .any(|identity| identity["provider"] == "google")
    );
    assert!(
        identities
            .iter()
            .any(|identity| identity["provider"] == "microsoft")
    );

    let repeated_start = send(&app, "POST", "/auth/link/microsoft", Some(&session_cookie)).await;
    let repeated_authorization = Url::parse(
        repeated_start.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture"),
    )
    .expect("provider URL parses");
    let repeated_query = |key: &str| {
        repeated_authorization
            .query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
            .expect("authorization query field exists")
    };
    let repeated_state = repeated_query("state");
    let repeated_nonce = repeated_query("nonce");
    let repeated_binding = repeated_start.headers()[header::SET_COOKIE]
        .to_str()
        .expect("valid test fixture")
        .split(';')
        .next()
        .expect("valid test fixture")
        .to_owned();
    let repeated_code = provider
        .token()
        .subject("linked-provider-subject")
        .nonce(&repeated_nonce)
        .issue();
    let repeated_link = send(
        &app,
        "GET",
        &callback_uri_for("microsoft", &repeated_state, Some(&repeated_code), None),
        Some(&repeated_binding),
    )
    .await;
    assert_eq!(repeated_link.status(), StatusCode::SEE_OTHER);
    assert_eq!(repeated_link.headers()[header::LOCATION], "/settings");

    let returning_sign_in = start_flow(&app, provider.issuer(), "/settings").await;
    let returning_code = provider
        .token()
        .subject("link-owner")
        .nonce(&returning_sign_in.nonce)
        .issue();
    let returning_session =
        complete_sign_in(&app, "google", &returning_sign_in, &returning_code).await;
    assert!(returning_session.starts_with("pv_session="));
}

#[sqlx::test]
async fn linking_an_identity_owned_by_another_account_changes_neither_account(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app_with_microsoft(pool, provider.issuer(), &root).await;

    let other_flow = start_flow_for(&app, provider.issuer(), "microsoft", "/settings").await;
    let other_code = provider
        .token()
        .subject("identity-already-owned")
        .nonce(&other_flow.nonce)
        .issue();
    let other_session = complete_sign_in(&app, "microsoft", &other_flow, &other_code).await;

    let owner_flow = start_flow(&app, provider.issuer(), "/settings").await;
    let owner_code = provider
        .token()
        .subject("different-owner")
        .nonce(&owner_flow.nonce)
        .issue();
    let owner_session = complete_sign_in(&app, "google", &owner_flow, &owner_code).await;

    let started = send(&app, "POST", "/auth/link/microsoft", Some(&owner_session)).await;
    assert_eq!(started.status(), StatusCode::SEE_OTHER);
    let authorization = Url::parse(
        started.headers()[header::LOCATION]
            .to_str()
            .expect("valid test fixture"),
    )
    .expect("provider URL parses");
    let query = |key: &str| {
        authorization
            .query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
            .expect("authorization query field exists")
    };
    let state = query("state");
    let nonce = query("nonce");
    let binding_cookie = started.headers()[header::SET_COOKIE]
        .to_str()
        .expect("valid test fixture")
        .split(';')
        .next()
        .expect("valid test fixture")
        .to_owned();
    let code = provider
        .token()
        .subject("identity-already-owned")
        .nonce(&nonce)
        .issue();

    let conflict = send(
        &app,
        "GET",
        &callback_uri_for("microsoft", &state, Some(&code), None),
        Some(&binding_cookie),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::SEE_OTHER);
    assert!(
        conflict.headers()[header::LOCATION]
            .to_str()
            .expect("location is valid")
            .starts_with("/auth/callback?status=conflict&provider=microsoft")
    );

    for (session, expected_provider) in [(&other_session, "microsoft"), (&owner_session, "google")]
    {
        let current = send(&app, "GET", "/api/me", Some(session)).await;
        assert_eq!(current.status(), StatusCode::OK);
        let body = axum::body::to_bytes(current.into_body(), 16 * 1024)
            .await
            .expect("small current-user response");
        let body: Value = serde_json::from_slice(&body).expect("valid current-user JSON");
        let identities = body["identities"].as_array().expect("identity list");
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0]["provider"], expected_provider);
    }
}

#[sqlx::test]
async fn unknown_and_unconfigured_provider_routes_remain_not_found(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app(pool, provider.issuer(), &root).await;

    assert_eq!(
        send(&app, "GET", "/auth/no-such-provider/start", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, "GET", "/auth/microsoft/start", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, "GET", "/auth/no-such-provider/callback?state=x", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn retired_provider_start_callback_and_link_requests_return_not_found(pool: PgPool) {
    let provider = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app(pool, provider.issuer(), &root).await;
    let flow = start_flow(&app, provider.issuer(), "/settings").await;
    let code = provider.token().nonce(&flow.nonce).issue();
    let session = complete_sign_in(&app, "google", &flow, &code).await;
    for (method, uri) in [
        ("GET", "/auth/facebook/start"),
        ("GET", "/auth/facebook/callback?state=unused&code=unused"),
        ("POST", "/auth/link/facebook"),
    ] {
        assert_eq!(
            send(&app, method, uri, Some(&session)).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        send_form(
            &app,
            "/auth/facebook/callback",
            Some(&session),
            "state=unused&code=unused"
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn verified_email_does_not_grant_admin(pool: PgPool) {
    let mock = support::MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app_with_providers(pool, mock.issuer(), &root, false).await;
    let flow = start_flow(&app, mock.issuer(), "/admin").await;
    let code = mock
        .token()
        .nonce(&flow.nonce)
        .claim("email", json!("bootstrap@gmail.com"))
        .claim("email_verified", json!(true))
        .issue();
    let cookie = complete_sign_in(&app, "google", &flow, &code).await;
    assert_eq!(
        send(&app, "GET", "/api/admin/users", Some(&cookie))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[cfg(debug_assertions)]
#[sqlx::test]
async fn provider_and_development_signins_apply_the_same_account_policy(pool: PgPool) {
    let mock = MockOidcProvider::start().await;
    let root = ScratchRoot::new();
    let app = app_with_providers(pool.clone(), mock.issuer(), &root, false).await;
    let subject = "bootstrap@gmail.com";
    let mut account_id = None;
    for development in [false, true, false] {
        if let Some(id) = account_id {
            sqlx::query("UPDATE users SET is_site_admin=false WHERE id=$1")
                .bind(id)
                .execute(&pool)
                .await
                .expect("remove role before sign-in");
            sqlx::query(
                "UPDATE identities SET last_seen_at=now()-interval '1 day' WHERE user_id=$1",
            )
            .bind(id)
            .execute(&pool)
            .await
            .expect("age activity");
        }
        let cookie = if development {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/auth/dev/login")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            json!({"subject":subject,"grant_admin":true}).to_string(),
                        ))
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            response.headers()[header::SET_COOKIE]
                .to_str()
                .expect("cookie")
                .split(';')
                .next()
                .expect("cookie pair")
                .to_owned()
        } else {
            let flow = start_flow(&app, mock.issuer(), "/admin").await;
            let code = mock
                .token()
                .subject(subject)
                .nonce(&flow.nonce)
                .claim("email", json!(subject))
                .claim("email_verified", json!(true))
                .issue();
            complete_sign_in(&app, "google", &flow, &code).await
        };
        let me = send(&app, "GET", "/api/me", Some(&cookie)).await;
        assert_eq!(me.status(), StatusCode::OK);
        let me: Value = serde_json::from_slice(
            &axum::body::to_bytes(me.into_body(), 16 * 1024)
                .await
                .expect("body"),
        )
        .expect("JSON");
        let id: Uuid = me["user"]["id"]
            .as_str()
            .expect("user id")
            .parse()
            .expect("UUID");
        assert_eq!(
            *account_id.get_or_insert(id),
            id,
            "both paths use the same account"
        );
        assert_eq!(me["user"]["is_site_admin"], development);
        assert_eq!(
            send(&app, "GET", "/api/admin/users", Some(&cookie))
                .await
                .status(),
            if development {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
        let seen_recently: bool = sqlx::query_scalar(
            "SELECT last_seen_at > now() - interval '1 minute' FROM identities WHERE user_id=$1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("identity activity");
        assert!(seen_recently, "both paths update identity activity");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .expect("accounts"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .expect("sessions"),
        3
    );
}
