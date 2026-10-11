use axum::{Json, body::to_bytes, extract::State};
use axum_extra::extract::cookie::CookieJar;
use sqlx::{PgPool, postgres::PgPoolOptions};

use super::{DevLoginRequest, login};
use crate::{
    AppState, auth::ProviderRegistry, config::Config, error::AppError, photos::worker::VariantQueue,
};

fn state(pool: PgPool, enabled: bool) -> AppState {
    let config: Config = serde_json::from_value(serde_json::json!({
        "database": {
            "url": "postgres://pinvoyager:pinvoyager@localhost/pinvoyager",
            "max_connections": 1,
            "min_connections": 0,
            "acquire_timeout_secs": 2
        },
        "auth": {
            "base_url": "http://localhost:8080/",
            "dev_login": enabled,
            "session": { "cookie_secure": false }
        }
    }))
    .expect("test configuration deserializes");

    AppState::new(
        pool,
        std::sync::Arc::new(config),
        ProviderRegistry::empty(),
        VariantQueue::detached(),
    )
}

fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("lazy pool accepts a syntactically valid URL")
}

async fn invoke(
    app: AppState,
    request: DevLoginRequest,
) -> Result<(CookieJar, axum::response::Response), AppError> {
    login(State(app), CookieJar::new(), Json(request)).await
}

#[tokio::test]
async fn a_disabled_dev_login_is_indistinguishable_from_a_missing_route() {
    let result = invoke(
        state(lazy_pool(), false),
        DevLoginRequest {
            display_name: None,
            subject: "person@example.test".to_owned(),
            provider: None,
        },
    )
    .await;

    assert!(matches!(result, Err(AppError::NotFound)));
}

#[tokio::test]
async fn blank_subject_and_unknown_provider_are_bad_requests() {
    let app = state(lazy_pool(), true);
    let blank = invoke(
        app.clone(),
        DevLoginRequest {
            display_name: None,
            subject: " \t ".to_owned(),
            provider: None,
        },
    )
    .await;
    assert!(
        matches!(blank, Err(AppError::BadRequest(message)) if message == "subject must not be empty")
    );

    let unknown = invoke(
        app,
        DevLoginRequest {
            display_name: None,
            subject: "person@example.test".to_owned(),
            provider: Some("unknown".to_owned()),
        },
    )
    .await;
    assert!(
        matches!(unknown, Err(AppError::BadRequest(message)) if message == "unknown provider: unknown")
    );
}

#[sqlx::test]
async fn first_and_returning_logins_issue_sessions_for_the_same_identity(pool: PgPool) {
    crate::db::migrate(&pool).await.expect("schema is migrated");
    let app = state(pool.clone(), true);
    let subject = format!("person-{}@example.test", uuid::Uuid::now_v7());
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .finish();

    let (first_jar, first_response) = {
        use tracing::instrument::WithSubscriber as _;
        invoke(
            app.clone(),
            DevLoginRequest {
                display_name: None,
                subject: subject.clone(),
                provider: Some("google".to_owned()),
            },
        )
        .with_subscriber(subscriber)
        .await
        .expect("first login creates the account")
    };
    assert!(first_jar.get("pv_session").is_some());
    let first_body = to_bytes(first_response.into_body(), usize::MAX)
        .await
        .expect("small login response");
    let first: serde_json::Value = serde_json::from_slice(&first_body).expect("valid test fixture");

    let (second_jar, second_response) = invoke(
        app,
        DevLoginRequest {
            display_name: None,
            subject: subject.clone(),
            provider: None,
        },
    )
    .await
    .expect("returning login finds the existing identity");
    assert!(second_jar.get("pv_session").is_some());
    let second_body = to_bytes(second_response.into_body(), usize::MAX)
        .await
        .expect("small login response");
    let second: serde_json::Value =
        serde_json::from_slice(&second_body).expect("valid test fixture");

    assert_eq!(first["provider"], "google");
    assert_eq!(second["provider"], "google");
    assert_eq!(first["subject"], subject);
    assert_eq!(first["user_id"], second["user_id"]);

    let user_id: uuid::Uuid = first["user_id"]
        .as_str()
        .expect("valid test fixture")
        .parse()
        .expect("valid test fixture");
    let display_name: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("read the stored display name");
    assert!(display_name.is_none());
}
