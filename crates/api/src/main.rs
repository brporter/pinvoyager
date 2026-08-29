//! Service entrypoint.
//!
//! Startup order is load-bearing: telemetry first so that configuration
//! failures are logged rather than silently swallowed, then configuration
//! (which fails loudly on anything missing), then the database, then the
//! listener.

use std::process::ExitCode;

use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::photos::worker;
use pinvoyager_api::{AppState, Config, db, http, telemetry};

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = run().await {
        // Telemetry may not be initialized yet, so this goes to stderr
        // directly. Losing a startup failure to an uninitialized subscriber is
        // a genuinely annoying way to spend an afternoon.
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

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::load()?;
    telemetry::init(&config.telemetry)?;

    tracing::info!(
        bind = %config.server.bind,
        "starting pinvoyager-api"
    );

    let pool = db::connect(&config.database).await?;
    db::migrate(&pool).await?;

    // Discovery happens once, here. A provider that fails to come up is logged
    // and disabled rather than fatal — one provider's outage must not stop the
    // service from serving the other three.
    // Third line of defense behind `#[cfg(debug_assertions)]` and the flag
    // itself. Loud because a forged-login endpoint reachable by accident is the
    // whole game, and a warning nobody notices is the same as no warning.
    #[cfg(debug_assertions)]
    if config.auth.dev_login {
        tracing::warn!("════════════════════════════════════════════════════════════════");
        tracing::warn!("DEV LOGIN IS ENABLED. POST /auth/dev/login forges a session for ");
        tracing::warn!("any identity, with no provider verification whatsoever. Anyone   ");
        tracing::warn!("who can reach this port can become any account.                  ");
        tracing::warn!("════════════════════════════════════════════════════════════════");
    }

    let providers = ProviderRegistry::from_config(&config.auth).await?;
    tracing::info!(
        providers = ?providers.configured(),
        "authentication providers ready"
    );

    // Shared before the state exists, because the variant worker needs the same
    // configuration and has to be constructed first — `AppState` holds its
    // queue handle.
    let config = std::sync::Arc::new(config);

    // Also sweeps any photos left `pending` by a previous run, which would
    // otherwise sit in someone's library forever, never becoming viewable.
    let variants = worker::spawn(pool.clone(), config.clone());

    let state = AppState::new(pool, config, providers, variants);
    http::serve(state).await?;

    Ok(())
}
