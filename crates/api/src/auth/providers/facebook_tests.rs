use super::*;
use crate::auth::AuthProvider;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::any;
use secrecy::SecretString;
use std::sync::{Arc, Mutex};

#[test]
fn facebook_authorization_url_reports_an_invalid_base() {
    let provider = configured_provider(Url::parse("http://127.0.0.1:1234").unwrap());
    let error = match provider.authorize_at("://invalid") {
        Ok(_) => panic!("malformed authorization endpoint should be rejected"),
        Err(error) => error,
    };

    assert!(
        matches!(error, AuthError::Configuration(message) if message.contains("Facebook authorize URL"))
    );
}

#[derive(Clone)]
struct GraphReply {
    token_status: StatusCode,
    token_body: String,
    profile_status: StatusCode,
    profile_body: String,
}

#[derive(Debug)]
struct SeenRequest {
    method: String,
    uri: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: String,
}

async fn serve_graph(
    State((reply, requests)): State<(GraphReply, Arc<Mutex<Vec<SeenRequest>>>)>,
    request: Request<Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX).await.expect("read request body");
    let uri = parts.uri.to_string();
    requests.lock().unwrap().push(SeenRequest {
        method: parts.method.to_string(),
        uri: uri.clone(),
        authorization: parts
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        content_type: parts
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        body: String::from_utf8_lossy(&body).into_owned(),
    });

    let (status, body) = if parts.method == "POST" && uri.starts_with("/v23.0/oauth/access_token") {
        (reply.token_status, reply.token_body)
    } else {
        (reply.profile_status, reply.profile_body)
    };

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("build mock response")
}

async fn mock_graph(
    reply: GraphReply,
) -> (
    Url,
    Arc<Mutex<Vec<SeenRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock Graph API");
    let address = listener.local_addr().expect("read mock server address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .fallback(any(serve_graph))
        .with_state((reply, requests.clone()));
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mock Graph API");
    });
    (
        Url::parse(&format!("http://{address}/v23.0/")).expect("mock Graph API URL"),
        requests,
        task,
    )
}

fn configured_provider(graph_root: Url) -> FacebookProvider {
    let config = FacebookProviderConfig {
        client_id: "facebook-client".to_owned(),
        client_secret: SecretString::from("facebook-secret"),
    };
    let base_url = Url::parse("https://pinvoyager.example").expect("valid base URL");
    let mut provider =
        FacebookProvider::new(&config, &base_url).expect("provider configuration is valid");
    provider.graph_api_root = graph_root;
    provider
}

fn provider() -> FacebookProvider {
    let config = FacebookProviderConfig {
        client_id: "facebook-client".to_owned(),
        client_secret: SecretString::from("facebook-secret"),
    };
    let base_url = Url::parse("https://pinvoyager.example").expect("valid base URL");
    FacebookProvider::new(&config, &base_url).expect("static provider configuration is valid")
}

#[test]
fn the_facebook_provider_has_its_own_kind_and_versioned_graph_endpoint() {
    let provider = provider();

    assert_eq!(provider.kind(), ProviderKind::Facebook);
    assert_eq!(
        provider.graph_url("me").expect("Graph URL"),
        Url::parse("https://graph.facebook.com/v23.0/me").unwrap()
    );
    assert_eq!(
        provider
            .graph_url("oauth/access_token")
            .expect("token endpoint")
            .path(),
        "/v23.0/oauth/access_token"
    );
}

#[test]
fn authorization_uses_state_pkce_and_only_the_public_profile_scope() {
    let provider = provider();
    let redirect = provider.authorize().expect("build authorize redirect");
    let query: std::collections::HashMap<_, _> = redirect.url.query_pairs().into_owned().collect();

    assert_eq!(redirect.url.scheme(), "https");
    assert_eq!(redirect.url.host_str(), Some("www.facebook.com"));
    assert_eq!(redirect.url.path(), "/v23.0/dialog/oauth");
    assert_eq!(
        query.get("client_id").map(String::as_str),
        Some("facebook-client")
    );
    assert_eq!(
        query.get("redirect_uri").map(String::as_str),
        Some("https://pinvoyager.example/auth/facebook/callback")
    );
    assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(
        query.get("scope").map(String::as_str),
        Some("public_profile")
    );
    assert!(!query.contains_key("email"));
    assert_eq!(query.get("state"), Some(&redirect.state));
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        query
            .get("code_challenge")
            .map(String::as_str)
            .unwrap()
            .len(),
        43
    );
    assert!(!redirect.pkce_verifier.is_empty());
    assert!(!redirect.nonce.is_empty());
}

#[tokio::test]
async fn exchange_sends_the_code_and_pkce_then_fetches_the_profile_with_the_token() {
    let (root, requests, task) = mock_graph(GraphReply {
        token_status: StatusCode::OK,
        token_body: r#"{"access_token":"short-lived-token","token_type":"bearer"}"#.into(),
        profile_status: StatusCode::OK,
        profile_body: r#"{"id":"app-scoped-42","name":"Ada Lovelace","picture":{"data":{"url":"https://images.example/ada.jpg","is_silhouette":false}}}"#.into(),
    })
    .await;
    let provider = configured_provider(root);

    let identity = provider
        .exchange("auth code +", "pkce verifier", "ignored nonce", None)
        .await
        .expect("exchange and profile request succeed");

    assert_eq!(identity.provider, ProviderKind::Facebook);
    assert_eq!(identity.subject, "app-scoped-42");
    assert_eq!(identity.display_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(
        identity.avatar_url.as_deref(),
        Some("https://images.example/ada.jpg")
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].uri, "/v23.0/oauth/access_token");
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    let token_form: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(requests[0].body.as_bytes())
            .into_owned()
            .collect();
    assert_eq!(
        token_form.get("client_id").map(String::as_str),
        Some("facebook-client")
    );
    assert_eq!(
        token_form.get("client_secret").map(String::as_str),
        Some("facebook-secret")
    );
    assert_eq!(
        token_form.get("redirect_uri").map(String::as_str),
        Some("https://pinvoyager.example/auth/facebook/callback")
    );
    assert_eq!(
        token_form.get("code").map(String::as_str),
        Some("auth code +")
    );
    assert_eq!(
        token_form.get("code_verifier").map(String::as_str),
        Some("pkce verifier")
    );
    assert_eq!(requests[1].method, "GET");
    assert!(
        requests[1]
            .uri
            .starts_with("/v23.0/me?fields=id%2Cname%2Cpicture.type%28large%29")
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer short-lived-token")
    );
    task.abort();
}

#[tokio::test]
async fn exchange_refuses_a_provider_error_status_before_parsing_its_body() {
    let (root, requests, task) = mock_graph(GraphReply {
        token_status: StatusCode::BAD_REQUEST,
        token_body: "not a token response".into(),
        profile_status: StatusCode::OK,
        profile_body: r#"{"id":"unused"}"#.into(),
    })
    .await;

    let error = configured_provider(root)
        .exchange("bad-code", "verifier", "nonce", None)
        .await
        .expect_err("a rejected code exchange must fail");

    assert!(
        matches!(error, AuthError::Provider(message) if message.contains("code exchange was rejected"))
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    task.abort();
}

#[tokio::test]
async fn exchange_rejects_an_unreadable_token_response() {
    let (root, requests, task) = mock_graph(GraphReply {
        token_status: StatusCode::OK,
        token_body: "not JSON".into(),
        profile_status: StatusCode::OK,
        profile_body: r#"{"id":"unused"}"#.into(),
    })
    .await;

    let error = configured_provider(root)
        .exchange("code", "verifier", "nonce", None)
        .await
        .expect_err("malformed token JSON must fail");

    assert!(
        matches!(error, AuthError::Provider(message) if message.contains("unreadable token response"))
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    task.abort();
}

#[tokio::test]
async fn exchange_reports_a_transport_failure() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve a local port");
    let address = listener.local_addr().expect("read reserved address");
    drop(listener);
    let graph_root =
        Url::parse(&format!("http://{address}/v23.0/")).expect("construct local Graph API URL");

    let error = configured_provider(graph_root)
        .exchange("code", "verifier", "nonce", None)
        .await
        .expect_err("connection refusal must fail exchange");

    assert!(
        matches!(error, AuthError::Provider(message) if message.contains("code exchange failed"))
    );
}

#[tokio::test]
async fn exchange_rejects_graph_errors_and_malformed_profile_responses() {
    for (status, body, expected) in [
        (StatusCode::BAD_GATEWAY, "{}", "Graph /me was rejected"),
        (StatusCode::OK, "not JSON", "unreadable Graph /me response"),
    ] {
        let (root, requests, task) = mock_graph(GraphReply {
            token_status: StatusCode::OK,
            token_body: r#"{"access_token":"short-lived-token"}"#.into(),
            profile_status: status,
            profile_body: body.into(),
        })
        .await;

        let error = configured_provider(root)
            .exchange("code", "verifier", "nonce", None)
            .await
            .expect_err("Graph API failure must fail sign-in");

        assert!(matches!(error, AuthError::Provider(message) if message.contains(expected)));
        assert_eq!(requests.lock().unwrap().len(), 2);
        task.abort();
    }
}

#[tokio::test]
async fn profile_mapping_drops_blank_names_and_placeholder_avatars() {
    for (profile_body, expected_name, expected_avatar) in [
        (
            r#"{"id":"id-1","name":"   ","picture":{"data":{"url":"https://images.example/default.png","is_silhouette":true}}}"#,
            None,
            None,
        ),
        (
            r#"{"id":"id-2","picture":{"data":{"url":"https://images.example/photo.png"}}}"#,
            None,
            Some("https://images.example/photo.png"),
        ),
        (
            r#"{"id":"id-3","name":"Ada","picture":null}"#,
            Some("Ada"),
            None,
        ),
    ] {
        let (root, _requests, task) = mock_graph(GraphReply {
            token_status: StatusCode::OK,
            token_body: r#"{"access_token":"short-lived-token"}"#.into(),
            profile_status: StatusCode::OK,
            profile_body: profile_body.into(),
        })
        .await;

        let identity = configured_provider(root)
            .exchange("code", "verifier", "nonce", None)
            .await
            .expect("profile mapping succeeds");

        assert_eq!(identity.display_name.as_deref(), expected_name);
        assert_eq!(identity.avatar_url.as_deref(), expected_avatar);
        task.abort();
    }
}
