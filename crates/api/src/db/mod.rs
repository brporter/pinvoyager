//! Database access.
//!
//! CONTRACT — `connect`, `migrate`, and `ping` signatures are fixed;
//! implementations and everything under `repo` are owned by the database agent.
//!
//! D8: every function here is `async`. Repository functions take an executor
//! (`impl sqlx::PgExecutor<'_>`) rather than the pool directly, so a caller can
//! pass either the pool or a transaction — which is what lets tests run inside
//! a transaction that rolls back, and lets phase 5 compose multi-statement work
//! without a second set of functions.
//!
//! ## Runtime-checked queries, for now
//!
//! Everything here uses `sqlx::query`/`sqlx::query_as` rather than the
//! `query!`/`query_as!` macros. The macros need either a live `DATABASE_URL` or
//! a committed `.sqlx` offline cache at *compile* time, and phase 2 has neither
//! — using them now would make the crate fail to build for anyone without a
//! database. Phase 3 creates the offline cache (`cargo sqlx prepare`) and
//! switches these call sites to the compile-time macros, which is the stated
//! goal; the query text moves across unchanged.

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

/// Builds the pool.
///
/// `acquire_timeout` must be set from config — see D8. The default is no
/// timeout, which turns pool exhaustion into requests that hang forever
/// instead of a 503.
pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, DbError> {
    let acquire_timeout = Duration::from_secs(config.acquire_timeout_secs);

    // The URL is deliberately absent from this log line: it carries the
    // password. Everything else about the pool is worth having in the log,
    // because "why did that request 503" is answered by these three numbers.
    tracing::info!(
        max_connections = config.max_connections,
        min_connections = config.min_connections,
        acquire_timeout_secs = config.acquire_timeout_secs,
        "connecting to postgres"
    );

    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        // D8. Without this, an exhausted pool parks the caller forever and the
        // symptom is a service that looks alive while serving nothing.
        .acquire_timeout(acquire_timeout)
        // `connect` (rather than `connect_lazy`) opens a connection here, so a
        // bad URL or an unreachable database is a loud startup failure instead
        // of a 503 on the first real request.
        .connect(config.url.expose_secret())
        .await
        .map_err(DbError::Connect)?;

    tracing::info!("database pool ready");

    Ok(pool)
}

/// Applies migrations from `crates/api/migrations`.
///
/// Note phase 8 moves this to a deploy step; it runs at startup only for local
/// development convenience, and that split should stay easy to make.
pub async fn migrate(pool: &PgPool) -> Result<(), DbError> {
    // The path is relative to `CARGO_MANIFEST_DIR`, and the migrations are
    // embedded in the binary at compile time — so the deployed artifact needs
    // no migration files alongside it when phase 8 moves this to a deploy step.
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(DbError::Migrate)?;

    tracing::info!("migrations applied");

    Ok(())
}

/// Cheap round trip, used by the readiness probe.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Deliberately the cheapest possible statement. A readiness probe runs on a
    // timer forever, so it must not touch application tables — that would make
    // probe cost scale with data, and would couple liveness of the process to
    // the health of a schema it may be in the middle of migrating.
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
