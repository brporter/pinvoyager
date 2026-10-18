//! Load configuration, initialize telemetry, connect and migrate PostgreSQL,
//! initialize providers, then start workers and HTTP. Shutdown drains HTTP and
//! workers before flushing telemetry. Early failures are written to stderr.

use std::process::ExitCode;

use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::photos::worker;
use pinvoyager_api::{AppState, Config, db, http, telemetry};

#[tokio::main]
async fn main() -> ExitCode {
    entrypoint(std::env::args_os().skip(1)).await
}

async fn entrypoint(args: impl IntoIterator<Item = std::ffi::OsString>) -> ExitCode {
    if let Err(err) = run(args).await {
        // Report startup failures even if telemetry initialization failed.
        eprintln!("fatal: {err}");
        let mut source = std::error::Error::source(&*err);
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(initial_admins) = pinvoyager_api::initial_admin::InitialAdmins::parse_args(args)?
    else {
        print!("{}", pinvoyager_api::initial_admin::HELP);
        return Ok(());
    };
    let mut config = Config::load()?;
    config.auth.initial_admins = initial_admins;
    let provider = telemetry::init(&config.telemetry)?;
    let result = run_service(config).await;
    telemetry::shutdown(provider).await;
    result
}

async fn run_service(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!(
        bind = %config.server.bind,
        "starting pinvoyager-api"
    );

    let pool = db::connect(&config.database).await?;
    db::migrate(&pool).await?;

    // Initialize providers independently; a discovery failure disables only
    // the affected provider. Warn if the development login bypass is enabled.
    #[cfg(debug_assertions)]
    if config.auth.dev_login {
        tracing::warn!("════════════════════════════════════════════════════════════════");
        tracing::warn!("DEV LOGIN IS ENABLED. POST /auth/dev/login forges a session for ");
        tracing::warn!("any identity, with no provider verification whatsoever. Anyone   ");
        tracing::warn!("who can reach this port can become any account.                  ");
        tracing::warn!("════════════════════════════════════════════════════════════════");
    }

    let providers = ProviderRegistry::from_config(&config.auth).await;
    let configured_providers = providers.configured();
    tracing::info!(
        providers = ?configured_providers,
        "authentication providers ready"
    );

    // Shared before the state exists, because the variant worker needs the same
    // configuration and has to be constructed first — `AppState` holds its
    // queue handle.
    let config = std::sync::Arc::new(config);

    // Also sweeps any photos left `pending` by a previous run, which would
    // otherwise remain in a voyage forever, never becoming viewable.
    let variants = worker::spawn(pool.clone(), config.clone());

    let deadline = std::time::Duration::from_secs(config.server.shutdown_timeout_secs);
    let state = AppState::new(pool, config, providers, variants.clone());
    let result = http::serve(state).await;
    variants.shutdown(deadline).await;
    result?;

    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
