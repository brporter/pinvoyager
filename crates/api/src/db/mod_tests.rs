use std::time::Duration;

use secrecy::SecretString;
use sqlx::postgres::PgPoolOptions;

use super::{DbError, connect, ping};
use crate::config::DatabaseConfig;

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        max_connections: 2,
        min_connections: 0,
        acquire_timeout_secs: 2,
    }
}

#[tokio::test]
async fn connects_to_postgres_and_answers_the_readiness_query() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required by the test setup");
    let pool = connect(&config(url))
        .await
        .expect("database connection succeeds");

    ping(&pool).await.expect("readiness query succeeds");

    pool.close().await;
}

#[tokio::test]
async fn connection_failures_are_reported_as_connect_errors_without_the_url() {
    let secret_url = "postgres://user:secret@127.0.0.1:1/not-a-database";
    let error = connect(&config(secret_url.to_owned()))
        .await
        .expect_err("unreachable database must fail startup");

    assert!(matches!(error, DbError::Connect(_)));
    assert_eq!(error.to_string(), "could not connect to the database");
    assert!(!format!("{error:?}").contains("secret"));
}

#[tokio::test]
async fn readiness_query_returns_database_errors() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://user@127.0.0.1:1/not-a-database")
        .expect("lazy pool accepts a syntactically valid URL");

    assert!(ping(&pool).await.is_err());
    pool.close().await;
}

#[sqlx::test]
async fn migration_failures_are_reported_without_discarding_the_cause(pool: sqlx::PgPool) {
    sqlx::query(
        "UPDATE _sqlx_migrations
            SET success = false
          WHERE version = (SELECT max(version) FROM _sqlx_migrations)",
    )
    .execute(&pool)
    .await
    .expect("the isolated database has completed migrations");

    let error = super::migrate(&pool)
        .await
        .expect_err("a dirty migration must not be treated as applied");

    assert!(matches!(error, super::DbError::Migrate(_)));
}
