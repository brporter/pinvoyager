use super::*;
use crate::auth::providers::apple::AppleProvider;
use crate::test_support::MockOidcProvider;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const NATIVE_ID: &str = "com.pinvoyager.ios";
const APP_STATE: &str = "native-apple-state-with-at-least-forty-three-characters";
const VERIFIER: &str = "native-apple-verifier-with-at-least-forty-three-characters";
const KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg5XYNJMqUWV4t8Ive\n\
        aMzdW9Hywi9Gm8seX4KiD21HUhWhRANCAATtgsS2FtzL4WAKAi3QEq+IOOigdivH\n\
        EwyiPEKo7DuSlwxiTqRmG18C/R+nl1hA1RBdGaijKfRjoQO9hyxWj7kq\n\
        -----END PRIVATE KEY-----\n";

async fn app(pool: sqlx::PgPool, mock: &MockOidcProvider, native: bool) -> Router {
    let mut apple = json!({"client_id":"website.services.id","team_id":"TEAM123456","key_id":"KEY1234567","private_key_pem":KEY});
    if native {
        apple["native_client_id"] = json!(NATIVE_ID);
    }
    let config: crate::Config = serde_json::from_value(json!({"database":{"url":"postgres://unused","max_connections":4,"min_connections":0,"acquire_timeout_secs":5},"auth":{"base_url":"https://pinvoyager.test","providers":{"apple":apple}}})).expect("valid test fixture");
    let provider = AppleProvider::discover(
        config
            .auth
            .providers
            .apple
            .as_ref()
            .expect("valid test fixture"),
        &config.auth.base_url,
        mock.issuer().as_str(),
    )
    .await
    .expect("valid test fixture");
    let router = crate::http::router(AppState::new(
        pool,
        std::sync::Arc::new(config),
        crate::auth::ProviderRegistry::for_test(ProviderKind::Apple, Box::new(provider)),
        crate::photos::worker::VariantQueue::detached(),
    ));
    let response = router
        .clone()
        .oneshot(
            Request::get("/api/client-capabilities")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body"),
    )
    .expect("capabilities");
    assert_eq!(body["native_apple_authentication"], native);
    router
}

async fn request(
    app: &Router,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::post(path).header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            builder
                .body(Body::from(body.to_string()))
                .expect("valid test fixture"),
        )
        .await
        .expect("valid test fixture");
    let status = response.status();
    if status.is_success() {
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("valid test fixture");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn begin(app: &Router, mode: &str, token: Option<&str>) -> Value {
    let (status, body) = request(app,"/auth/native/apple/attempts",json!({"mode":mode,"state":APP_STATE,"challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER))}),token).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["state"], APP_STATE);
    assert!(body["nonce"].as_str().expect("valid test fixture").len() >= 43);
    body
}
async fn complete(app: &Router, attempt: &Value, code: &str) -> (StatusCode, Value) {
    request(app,"/auth/native/apple/complete",json!({"attempt_id":attempt["attempt_id"],"state":APP_STATE,"verifier":VERIFIER,"authorization_code":code}),None).await
}
#[sqlx::test(migrations = "./migrations")]
async fn native_apple_reuses_the_web_identity_and_sessions_are_single_use(pool: sqlx::PgPool) {
    let mock = MockOidcProvider::start().await;
    let owner = crate::db::repo::users::create(&pool, None, None)
        .await
        .expect("valid test fixture");
    crate::db::repo::identities::create(
        &pool,
        owner.id,
        crate::db::repo::identities::Provider::Apple,
        "same-grouped-subject",
    )
    .await
    .expect("valid test fixture");
    let app = app(pool.clone(), &mock, true).await;
    let attempt = begin(&app, "sign_in", None).await;
    let code = mock
        .token()
        .audience(NATIVE_ID)
        .nonce(attempt["nonce"].as_str().expect("valid test fixture"))
        .subject("same-grouped-subject")
        .issue();
    let (status, session) = complete(&app, &attempt, &code).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(
        complete(&app, &attempt, &code).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(session["user"]["id"], owner.id.to_string());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("valid test fixture");
    assert_eq!(count, 1);
    assert!(mock.revocations().is_empty());
    let requests = mock.token_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["client_id"], NATIVE_ID);
    assert!(!requests[0].contains_key("redirect_uri"));
    assert!(!requests[0].contains_key("code_verifier"));
    let claims = requests[0]["client_secret"]
        .split('.')
        .nth(1)
        .expect("valid test fixture");
    let claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).expect("valid test fixture"))
            .expect("valid test fixture");
    assert_eq!(claims["sub"], NATIVE_ID);
}

#[sqlx::test(migrations = "./migrations")]
async fn native_apple_rejects_bad_state_pkce_nonce_audience_expiry_and_signature(
    pool: sqlx::PgPool,
) {
    let mock = MockOidcProvider::start().await;
    let app = app(pool.clone(), &mock, true).await;
    for failure in [
        "state",
        "verifier",
        "nonce",
        "audience",
        "expired",
        "signature",
        "issuer",
        "code",
        "attempt",
    ] {
        let attempt = begin(&app, "sign_in", None).await;
        let mut token = mock
            .token()
            .audience(NATIVE_ID)
            .nonce(attempt["nonce"].as_str().expect("valid test fixture"));
        token = match failure {
            "nonce" => token.nonce("wrong"),
            "audience" => token.audience("website.services.id"),
            "expired" => token.expired(),
            "signature" => token.signed_by_a_different_key(),
            "issuer" => token.issuer("https://attacker.test"),
            _ => token,
        };
        let code = token.issue();
        let mut body = json!({"attempt_id":attempt["attempt_id"],"state":APP_STATE,"verifier":VERIFIER,"authorization_code":code});
        match failure {
            "state" => body["state"] = json!("wrong"),
            "verifier" => {
                body["verifier"] = json!("different-verifier-with-at-least-forty-three-characters")
            }
            "code" => body["authorization_code"] = json!("unknown-code"),
            "attempt" => {
                sqlx::query("UPDATE native_auth_attempts SET expires_at=now()-interval '1 second' WHERE id=$1").bind(attempt["attempt_id"].as_str().expect("valid test fixture").parse::<Uuid>().expect("valid test fixture")).execute(&pool).await.expect("valid test fixture");
            }
            _ => {}
        }
        assert_eq!(
            request(&app, "/auth/native/apple/complete", body, None)
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{failure}"
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .expect("valid test fixture");
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn native_apple_linking_and_deletion_are_bound_to_the_initiating_account(pool: sqlx::PgPool) {
    let mock = MockOidcProvider::start().await;
    let app = app(pool.clone(), &mock, true).await;
    let first = begin(&app, "sign_in", None).await;
    let code = mock
        .token()
        .audience(NATIVE_ID)
        .nonce(first["nonce"].as_str().expect("valid test fixture"))
        .subject("first-subject")
        .issue();
    let (_, session) = complete(&app, &first, &code).await;
    let token = session["session_token"]
        .as_str()
        .expect("valid test fixture");
    let owner: Uuid = session["user"]["id"]
        .as_str()
        .expect("valid test fixture")
        .parse()
        .expect("valid test fixture");
    let link = begin(&app, "link", Some(token)).await;
    let code = mock
        .token()
        .audience(NATIVE_ID)
        .nonce(link["nonce"].as_str().expect("valid test fixture"))
        .subject("first-subject")
        .issue();
    assert_eq!(complete(&app, &link, &code).await.0, StatusCode::OK);
    for subject in ["wrong-subject", "first-subject"] {
        let attempt = begin(&app, "reauth_delete", Some(token)).await;
        let code = mock
            .token()
            .audience(NATIVE_ID)
            .nonce(attempt["nonce"].as_str().expect("valid test fixture"))
            .subject(subject)
            .issue();
        let (status, proof) = complete(&app, &attempt, &code).await;
        if subject == "wrong-subject" {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(mock.revocations().is_empty());
        } else {
            assert_eq!(status, StatusCode::OK);
            assert_eq!(proof["account_id"], owner.to_string());
            assert_eq!(proof["purpose"], "account_deletion");
            assert!(proof.get("session_token").is_none());
        }
    }
    assert_eq!(mock.revocations().len(), 1);
    assert_eq!(mock.revocations()[0]["client_id"], NATIVE_ID);
}

#[sqlx::test(migrations = "./migrations")]
async fn native_apple_requires_configuration_and_authenticated_linking(pool: sqlx::PgPool) {
    let mock = MockOidcProvider::start().await;
    let disabled = app(pool.clone(), &mock, false).await;
    assert_eq!(request(&disabled,"/auth/native/apple/attempts",json!({"mode":"sign_in","state":APP_STATE,"challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER))}),None).await.0,StatusCode::NOT_FOUND);
    let app = app(pool, &mock, true).await;
    for mode in ["link", "reauth_delete"] {
        assert_eq!(request(&app,"/auth/native/apple/attempts",json!({"mode":mode,"state":APP_STATE,"challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER))}),None).await.0,StatusCode::UNAUTHORIZED);
    }
    for body in [
        json!({"mode":"invalid","state":APP_STATE,"challenge":"invalid"}),
        json!({"mode":"sign_in","state":"short","challenge":"invalid"}),
    ] {
        assert_eq!(
            request(&app, "/auth/native/apple/attempts", body, None)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn native_apple_consumes_concurrent_requests_and_rejects_browser_bootstrap(
    pool: sqlx::PgPool,
) {
    let mock = MockOidcProvider::start().await;
    let app = app(pool.clone(), &mock, true).await;
    let attempt = begin(&app, "sign_in", None).await;
    let id: Uuid = attempt["attempt_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("UUID");
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/auth/native/attempts/{id}/start"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let code = mock
        .token()
        .audience(NATIVE_ID)
        .nonce(attempt["nonce"].as_str().expect("nonce"))
        .issue();
    let (left, right) = tokio::join!(
        complete(&app, &attempt, &code),
        complete(&app, &attempt, &code)
    );
    assert!(matches!(
        (left.0, right.0),
        (StatusCode::OK, StatusCode::BAD_REQUEST) | (StatusCode::BAD_REQUEST, StatusCode::OK)
    ));
    assert_eq!(mock.token_requests().len(), 1);
    for (verifier, code) in [
        ("short", "code"),
        (VERIFIER, ""),
        (VERIFIER, &"a".repeat(4097)),
    ] {
        assert_eq!(request(&app,"/auth/native/apple/complete",json!({"attempt_id":id,"state":APP_STATE,"verifier":verifier,"authorization_code":code}),None).await.0,StatusCode::BAD_REQUEST);
    }
    for challenge in ["invalid".to_owned(), URL_SAFE_NO_PAD.encode([0u8; 31])] {
        assert_eq!(
            request(
                &app,
                "/auth/native/apple/attempts",
                json!({"mode":"sign_in","state":APP_STATE,"challenge":challenge}),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}
