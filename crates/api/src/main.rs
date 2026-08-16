//! Service entrypoint.
//!
//! Startup order is load-bearing: telemetry first so that configuration
//! failures are logged rather than silently swallowed, then configuration
//! (which fails loudly on anything missing), then the database, then the
//! listener.

use std::process::ExitCode;

use pinvoyager_api::auth::ProviderRegistry;
use pinvoyager_api::{AppState, Config, db, http, telemetry};

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = run().await {
        // Telemetry may not be initialised yet, so this goes to stderr
        // directly. Losing a startup failure to an uninitialised subscriber is
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
    let providers = ProviderRegistry::from_config(&config.auth).await?;
    tracing::info!(
        providers = ?providers.configured(),
        "authentication providers ready"
    );

    let state = AppState::new(pool, config, providers);
    http::serve(state).await?;

    Ok(())
}
