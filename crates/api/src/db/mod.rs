//! Async database setup and migrations. Repository operations accept a pool or
//! transaction executor. Checked queries use committed .sqlx metadata; dynamic
//! queries are verified by database integration tests.

pub mod repo;

use std::time::Duration;

use secrecy::ExposeSecret;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::config::DatabaseConfig;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("could not connect to the database")]
    Connect(#[source] sqlx::Error),
    #[error("migrations failed")]
    Migrate(#[source] sqlx::migrate::MigrateError),
}

/// Connects a PostgreSQL pool using the configured connection and acquisition
/// limits. Connection failures return a database error.
pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, DbError> {
    let acquire_timeout = Duration::from_secs(config.acquire_timeout_secs);

    // Log pool limits without exposing the password in the connection URL.
    tracing::info!(
        max_connections = config.max_connections,
        min_connections = config.min_connections,
        acquire_timeout_secs = config.acquire_timeout_secs,
        "connecting to postgres"
    );

    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        // Limit how long requests wait for a pooled connection.
        .acquire_timeout(acquire_timeout)
        // Connect now so startup reports database connection failures.
        .connect(config.url.expose_secret())
        .await
        .map_err(DbError::Connect)?;

    tracing::info!("database pool ready");

    Ok(pool)
}

/// Applies migrations from `crates/api/migrations`.
///
pub async fn migrate(pool: &PgPool) -> Result<(), DbError> {
    // The path is relative to `CARGO_MANIFEST_DIR`, and the migrations are
    // embedded in the binary at compile time — so the deployed artifact needs
    // no migration files alongside it.
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(DbError::Migrate)?;

    tracing::info!("migrations applied");

    Ok(())
}

/// Runs `SELECT 1` for readiness, returning any database failure.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Check connectivity without reading application tables.
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
