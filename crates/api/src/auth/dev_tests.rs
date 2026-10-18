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

    sqlx::query("UPDATE identities SET last_seen_at = now() - interval '1 day' WHERE subject=$1")
        .bind(&subject)
        .execute(&pool)
        .await
        .expect("age last sign-in");

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
    let seen_recently: bool = sqlx::query_scalar(
        "SELECT last_seen_at > now() - interval '1 minute' FROM identities WHERE subject=$1",
    )
    .bind(&subject)
    .fetch_one(&pool)
    .await
    .expect("last sign-in");
    assert!(
        seen_recently,
        "forged sign-in updates identity activity like provider sign-in"
    );
}

#[sqlx::test]
async fn session_failure_rolls_back_new_accounts_and_existing_account_changes(pool: PgPool) {
    let mut app = state(pool.clone(), true);
    std::sync::Arc::get_mut(&mut app.config)
        .expect("unshared config")
        .auth
        .initial_admins =
        crate::initial_admin::InitialAdmins::parse_args([std::ffi::OsString::from(
            "--initial-admin-email=admin@example.test",
        )])
        .expect("arguments")
        .expect("run");
    sqlx::query("ALTER TABLE sessions ADD CONSTRAINT reject_test_sessions CHECK (false)")
        .execute(&pool)
        .await
        .expect("reject session inserts");
    let request = || DevLoginRequest {
        subject: "admin@example.test".to_owned(),
        provider: None,
        display_name: None,
    };
    assert!(matches!(
        invoke(app.clone(), request()).await,
        Err(AppError::Database(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .expect("accounts"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identities")
            .fetch_one(&pool)
            .await
            .expect("identities"),
        0
    );

    let account = crate::db::repo::users::create(&pool, Some("kept-name"), Some("kept-avatar"))
        .await
        .expect("existing account");
    let identity = crate::db::repo::identities::create(
        &pool,
        account.id,
        crate::db::repo::identities::Provider::Google,
        "admin@example.test",
    )
    .await
    .expect("identity");
    let old_activity: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE identities SET last_seen_at=now()-interval '1 day' WHERE id=$1 RETURNING last_seen_at")
        .bind(identity.id).fetch_one(&pool).await.expect("age activity");
    assert!(matches!(
        invoke(app, request()).await,
        Err(AppError::Database(_))
    ));
    let (is_admin, name, avatar): (bool, Option<String>, Option<String>) =
        sqlx::query_as("SELECT is_site_admin, display_name, avatar_url FROM users WHERE id=$1")
            .bind(account.id)
            .fetch_one(&pool)
            .await
            .expect("account");
    assert!(!is_admin, "failed sign-in must roll back promotion");
    assert_eq!(name.as_deref(), Some("kept-name"));
    assert_eq!(avatar.as_deref(), Some("kept-avatar"));
    assert_eq!(
        sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT last_seen_at FROM identities WHERE id=$1"
        )
        .bind(identity.id)
        .fetch_one(&pool)
        .await
        .expect("activity"),
        old_activity
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .expect("sessions"),
        0
    );
}
