//! Native handoff through the real provider verification and server session pipeline.
#[path = "support/contract_fixtures.rs"]
mod contracts;
mod support;

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use pinvoyager_api::{
    Config, auth::ProviderRegistry, http, photos::worker::VariantQueue, state::AppState,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tower::ServiceExt as _;
use url::Url;
use uuid::Uuid;

const APP_STATE: &str = "app-state-which-is-independent-of-the-provider-state-1234";
const VERIFIER: &str = "native-pkce-verifier-with-at-least-forty-three-characters-1234";

async fn app(pool: PgPool, provider: &support::MockOidcProvider) -> Router {
    let config: Config = serde_json::from_value(json!({
        "database": {"url":"postgres://unused","max_connections":4,"min_connections":1,"acquire_timeout_secs":5},
        "auth": {"base_url":"http://127.0.0.1:8080", "providers": {"google": {
            "client_id":support::CLIENT_ID, "client_secret":support::CLIENT_SECRET,
            "issuer":provider.issuer().as_str().trim_end_matches('/')
        }}}
    })).expect("test configuration");
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
    path: &str,
    body: Option<Value>,
    bearer: Option<&str>,
    cookie: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let body = if let Some(body) = body {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    app.clone()
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("router")
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body"),
    )
    .expect("JSON")
}

struct Attempt {
    id: Uuid,
    browser_path: String,
}

async fn attempt(app: &Router, mode: &str, bearer: Option<&str>) -> Attempt {
    let response = send(
        app,
        "POST",
        "/auth/native/attempts",
        Some(json!({
            "provider":"google", "mode":mode, "state":APP_STATE,
            "challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
            "callback_identifier":"pinvoyager"
        })),
        bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = json_body(response).await;
    let url = Url::parse(body["browser_start_url"].as_str().expect("start URL")).expect("URL");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    Attempt {
        id: body["attempt_id"]
            .as_str()
            .expect("id")
            .parse()
            .expect("UUID"),
        browser_path: url.path().to_owned(),
    }
}

async fn handoff(
    app: &Router,
    provider: &support::MockOidcProvider,
    attempt: &Attempt,
    subject: &str,
) -> String {
    let started = send(app, "GET", &attempt.browser_path, None, None, None).await;
    assert_eq!(started.status(), StatusCode::FOUND);
    let cookie = started.headers()[header::SET_COOKIE]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .to_owned();
    let url = Url::parse(
        started.headers()[header::LOCATION]
            .to_str()
            .expect("location"),
    )
    .expect("URL");
    let parameter = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .expect("parameter")
            .1
            .into_owned()
    };
    let code = provider
        .token()
        .nonce(&parameter("nonce"))
        .subject(subject)
        .issue();
    let path = format!(
        "/auth/google/callback?state={}&code={code}",
        parameter("state")
    );
    let response = send(app, "GET", &path, None, None, Some(&cookie)).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        !response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().expect("cookie").starts_with("pv_session="))
    );
    let callback = Url::parse(
        response.headers()[header::LOCATION]
            .to_str()
            .expect("location"),
    )
    .expect("callback URL");
    assert_eq!(callback.scheme(), "pinvoyager");
    assert_eq!(callback.host_str(), Some("auth"));
    assert_eq!(callback.path(), "/callback");
    assert_eq!(
        callback
            .query_pairs()
            .find(|(key, _)| key == "state")
            .expect("state")
            .1,
        APP_STATE
    );
    callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .expect("code")
        .1
        .into_owned()
}

async fn exchange(
    app: &Router,
    attempt: &Attempt,
    code: &str,
    verifier: &str,
) -> axum::response::Response {
    send(
        app,
        "POST",
        "/auth/native/exchange",
        Some(json!({"attempt_id":attempt.id,"code":code,"verifier":verifier})),
        None,
        None,
    )
    .await
}

#[sqlx::test]
async fn native_handoff_is_single_use_and_reuses_the_provider_account(pool: PgPool) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let capabilities = send(&app, "GET", "/api/client-capabilities", None, None, None).await;
    assert_eq!(capabilities.status(), StatusCode::OK);
    let capabilities = json_body(capabilities).await;
    assert_eq!(capabilities["native_authentication"], true);
    assert_eq!(capabilities["providers"], json!(["google"]));
    let first = attempt(&app, "sign_in", None).await;
    let code = handoff(&app, &provider, &first, "native-owner").await;
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .expect("sessions");
    assert_eq!(sessions, 0, "a handoff is not an app credential");
    let response = exchange(&app, &first, &code, VERIFIER).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let session = json_body(response).await;
    let mut fixture = session.clone();
    fixture["session_token"] = json!("synthetic-native-session-for-contract-checks");
    contracts::write_fixture("native_session_exchange", &fixture.to_string());
    let token = session["session_token"].as_str().expect("opaque session");
    let me = send(&app, "GET", "/api/me", None, Some(token), None).await;
    assert_eq!(me.status(), StatusCode::OK);
    assert_eq!(json_body(me).await["user"], session["user"]);
    assert!(session["idle_deadline"].is_string() && session["absolute_deadline"].is_string());
    assert_eq!(
        exchange(&app, &first, &code, VERIFIER).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&app, "GET", &first.browser_path, None, None, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let second = attempt(&app, "sign_in", None).await;
    let code = handoff(&app, &provider, &second, "native-owner").await;
    let again = json_body(exchange(&app, &second, &code, VERIFIER).await).await;
    assert_eq!(again["user"]["id"], session["user"]["id"]);
    assert_ne!(again["session_token"], session["session_token"]);
}

#[sqlx::test]
async fn native_code_requires_its_verifier_attempt_and_expiry(pool: PgPool) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let first = attempt(&app, "sign_in", None).await;
    let code = handoff(&app, &provider, &first, "pkce-owner").await;
    assert_eq!(
        exchange(
            &app,
            &first,
            &code,
            "wrong-verifier-with-at-least-forty-three-characters"
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let other = attempt(&app, "sign_in", None).await;
    assert_eq!(
        exchange(&app, &other, &code, VERIFIER).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        exchange(&app, &first, "wrong-code", VERIFIER)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query(
        "UPDATE native_auth_attempts SET code_expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(first.id)
    .execute(&pool)
    .await
    .expect("expire code");
    assert_eq!(
        exchange(&app, &first, &code, VERIFIER).await.status(),
        StatusCode::BAD_REQUEST
    );
    let expired = attempt(&app, "sign_in", None).await;
    sqlx::query("UPDATE native_auth_attempts SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(expired.id)
        .execute(&pool)
        .await
        .expect("expire attempt");
    assert_eq!(
        send(&app, "GET", &expired.browser_path, None, None, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn native_sessions_reject_mixed_credentials_and_logout_is_isolated(pool: PgPool) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let flow = attempt(&app, "sign_in", None).await;
    let code = handoff(&app, &provider, &flow, "session-owner").await;
    let session = json_body(exchange(&app, &flow, &code, VERIFIER).await).await;
    let token = session["session_token"].as_str().expect("session");
    let user: Uuid = session["user"]["id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("UUID");
    let browser = pinvoyager_api::auth::session::create(
        &pool,
        user,
        &pinvoyager_api::config::SessionConfig::default(),
    )
    .await
    .expect("browser session");
    let cookie = format!("pv_session={}", browser.expose());
    assert_eq!(
        send(&app, "GET", "/api/me", None, Some(token), Some(&cookie))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/voyages",
            None,
            Some("invalid"),
            Some(&cookie)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", "/api/me", None, Some(browser.expose()), None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/me",
            None,
            None,
            Some(&format!("pv_session={token}"))
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..2 {
        assert_eq!(
            send(&app, "POST", "/auth/native/logout", None, Some(token), None)
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        send(&app, "GET", "/api/me", None, Some(token), None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", "/api/me", None, None, Some(&cookie))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/auth/native/logout",
            None,
            Some(browser.expose()),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, "GET", "/api/me", None, None, Some(&cookie))
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn native_attempt_rejects_untrusted_callbacks_invalid_state_and_unbound_linking(
    pool: PgPool,
) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool, &provider).await;
    let body = json!({"provider":"google","mode":"sign_in","state":APP_STATE,"challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),"callback_identifier":"pinvoyager"});
    for (key, value) in [
        ("callback_identifier", "https://evil.test"),
        ("state", "short"),
        ("challenge", "invalid"),
        ("mode", "unknown"),
    ] {
        let mut invalid = body.clone();
        invalid[key] = json!(value);
        assert_eq!(
            send(
                &app,
                "POST",
                "/auth/native/attempts",
                Some(invalid),
                None,
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut link = body.clone();
    link["mode"] = json!("link");
    assert_eq!(
        send(
            &app,
            "POST",
            "/auth/native/attempts",
            Some(link),
            None,
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let flow = attempt(&app, "sign_in", None).await;
    let started = send(&app, "GET", &flow.browser_path, None, None, None).await;
    let url = Url::parse(
        started.headers()[header::LOCATION]
            .to_str()
            .expect("location"),
    )
    .expect("URL");
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .expect("state")
        .1
        .into_owned();
    let nonce = url
        .query_pairs()
        .find(|(key, _)| key == "nonce")
        .expect("nonce")
        .1
        .into_owned();
    let code = provider.token().nonce(&nonce).issue();
    let callback = send(
        &app,
        "GET",
        &format!("/auth/google/callback?state={state}&code={code}"),
        None,
        None,
        None,
    )
    .await;
    assert!(
        !callback.headers()[header::LOCATION]
            .to_str()
            .expect("location")
            .starts_with("pinvoyager:")
    );
}

#[sqlx::test]
async fn deletion_requires_recent_purpose_proof_and_a_persisted_status_capability(pool: PgPool) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let login = attempt(&app, "sign_in", None).await;
    let code = handoff(&app, &provider, &login, "delete-owner").await;
    let session = json_body(exchange(&app, &login, &code, VERIFIER).await).await;
    let token = session["session_token"].as_str().expect("token");
    let user: Uuid = session["user"]["id"]
        .as_str()
        .expect("user")
        .parse()
        .expect("UUID");
    let response=send(&app,"POST","/auth/native/reauth-attempts",Some(json!({"provider":"google","purpose":"account_deletion","state":APP_STATE,"challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),"callback_identifier":"pinvoyager"})),Some(token),None).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let reply = json_body(response).await;
    let reauth = Attempt {
        id: reply["attempt_id"]
            .as_str()
            .expect("id")
            .parse()
            .expect("UUID"),
        browser_path: Url::parse(reply["browser_start_url"].as_str().expect("url"))
            .expect("url")
            .path()
            .to_owned(),
    };
    let code = handoff(&app, &provider, &reauth, "delete-owner").await;
    let proof = json_body(exchange(&app, &reauth, &code, VERIFIER).await).await;
    assert!(proof["session_token"].is_null());
    let operation = Uuid::now_v7();
    let capability = "deletion-status-capability-generated-before-preparation-1234";
    let request = json!({"proof_token":proof["proof_token"],"status_capability":capability});
    let prepare = || {
        Request::builder()
            .method("POST")
            .uri("/api/me/deletion-requests")
            .header("authorization", format!("Bearer {token}"))
            .header("idempotency-key", operation.to_string())
            .header("content-type", "application/json")
            .body(Body::from(request.to_string()))
            .expect("request")
    };
    let response = app.clone().oneshot(prepare()).await.expect("router");
    assert_eq!(response.status(), StatusCode::CREATED);
    let receipt = json_body(response).await;
    let receipt_id: Uuid = receipt["id"].as_str().expect("id").parse().expect("UUID");
    assert_eq!(
        json_body(app.clone().oneshot(prepare()).await.expect("router")).await,
        receipt
    );
    assert!(
        pinvoyager_api::db::repo::users::find_by_id(&pool, user)
            .await
            .expect("user")
            .is_some()
    );
    let confirm = send(
        &app,
        "POST",
        &format!("/api/me/deletion-requests/{receipt_id}/confirm"),
        None,
        Some(token),
        None,
    )
    .await;
    assert_eq!(confirm.status(), StatusCode::ACCEPTED);
    assert!(
        pinvoyager_api::db::repo::users::find_by_id(&pool, user)
            .await
            .expect("deleted")
            .is_none()
    );
    assert_eq!(
        send(&app, "GET", "/api/me", None, Some(token), None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let status = Request::builder()
        .uri(format!("/api/account-deletions/{receipt_id}"))
        .header("authorization", format!("Deletion {capability}"))
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(status).await.expect("router");
    assert_eq!(response.status(), StatusCode::OK);
    let status = json_body(response).await;
    assert_eq!(status["status"], "complete");
    assert!(status["user"].is_null());
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/api/account-deletions/{receipt_id}"),
            None,
            None,
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn native_linking_binds_the_initiating_account_and_expired_native_attempts_never_sign_into_the_browser(
    pool: PgPool,
) {
    use pinvoyager_api::{auth::session, db::repo::users};
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let user = users::create(&pool, Some("native-link-owner"), None)
        .await
        .expect("owner");
    pinvoyager_api::db::repo::identities::create(
        &pool,
        user.id,
        pinvoyager_api::db::repo::identities::Provider::Apple,
        "existing-apple-owner",
    )
    .await
    .expect("identity");
    let config = pinvoyager_api::config::SessionConfig::default();
    let token = session::create(&pool, user.id, &config)
        .await
        .expect("session");
    sqlx::query("INSERT INTO native_sessions(token_hash) VALUES($1)")
        .bind(token.hash())
        .execute(&pool)
        .await
        .expect("native session");
    let linked = attempt(&app, "link", Some(token.expose())).await;
    let code = handoff(&app, &provider, &linked, "newly-linked-google").await;
    let response = exchange(&app, &linked, &code, VERIFIER).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = json_body(response).await;
    assert_eq!(response["user"]["id"], user.id.to_string());
    assert_eq!(
        response["identities"].as_array().expect("identities").len(),
        2
    );
    let expired = attempt(&app, "sign_in", None).await;
    let started = send(&app, "GET", &expired.browser_path, None, None, None).await;
    let cookie = started.headers()[header::SET_COOKIE]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .to_owned();
    let url = Url::parse(
        started.headers()[header::LOCATION]
            .to_str()
            .expect("location"),
    )
    .expect("URL");
    let parameter = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .expect("parameter")
            .1
            .into_owned()
    };
    sqlx::query("UPDATE native_auth_attempts SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(expired.id)
        .execute(&pool)
        .await
        .expect("expire");
    let provider_code = provider
        .token()
        .nonce(&parameter("nonce"))
        .subject("expired-native-user")
        .issue();
    let response = send(
        &app,
        "GET",
        &format!(
            "/auth/google/callback?state={}&code={provider_code}",
            parameter("state")
        ),
        None,
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        !response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|value| value.to_str().expect("cookie").starts_with("pv_session="))
    );
    let exists:bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM identities WHERE provider='google' AND subject='expired-native-user')").fetch_one(&pool).await.expect("identity");
    assert!(!exists);
}

#[sqlx::test]
async fn native_exchange_rechecks_suspension_and_rejects_malformed_or_unknown_handoffs(
    pool: PgPool,
) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let flow = attempt(&app, "sign_in", None).await;
    let code = handoff(&app, &provider, &flow, "suspended-before-exchange").await;
    for (code, verifier) in [(code.as_str(), "short"), (&"x".repeat(129), VERIFIER)] {
        assert_eq!(
            exchange(&app, &flow, code, verifier).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    let unknown = Attempt {
        id: Uuid::now_v7(),
        browser_path: String::new(),
    };
    assert_eq!(
        exchange(&app, &unknown, &code, VERIFIER).await.status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE users SET suspended=true WHERE id=(SELECT user_id FROM native_auth_attempts WHERE id=$1)")
        .bind(flow.id).execute(&pool).await.expect("suspend");
    assert_eq!(
        exchange(&app, &flow, &code, VERIFIER).await.status(),
        StatusCode::FORBIDDEN
    );
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .expect("sessions");
    assert_eq!(sessions, 0);
    sqlx::query("UPDATE users SET suspended=false WHERE id=(SELECT user_id FROM native_auth_attempts WHERE id=$1)")
        .bind(flow.id).execute(&pool).await.expect("restore account");
    assert_eq!(
        exchange(&app, &flow, &code, VERIFIER).await.status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn native_attempts_reject_unconfigured_providers_and_unrelated_reauthentication_purposes(
    pool: PgPool,
) {
    let provider = support::MockOidcProvider::start().await;
    let app = app(pool.clone(), &provider).await;
    let base = json!({"provider":"google","mode":"sign_in","state":APP_STATE,"challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),"callback_identifier":"pinvoyager"});
    for (provider, status) in [
        ("apple", StatusCode::NOT_FOUND),
        ("unknown", StatusCode::NOT_FOUND),
    ] {
        let mut body = base.clone();
        body["provider"] = json!(provider);
        assert_eq!(
            send(
                &app,
                "POST",
                "/auth/native/attempts",
                Some(body),
                None,
                None
            )
            .await
            .status(),
            status
        );
    }
    for purpose in ["link", "sign_in", "", "account_deletion "] {
        let mut body = base.clone();
        body.as_object_mut().expect("object").remove("mode");
        body["purpose"] = json!(purpose);
        assert_eq!(
            send(
                &app,
                "POST",
                "/auth/native/reauth-attempts",
                Some(body),
                None,
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM native_auth_attempts")
        .fetch_one(&pool)
        .await
        .expect("attempts");
    assert_eq!(count, 0);
}
