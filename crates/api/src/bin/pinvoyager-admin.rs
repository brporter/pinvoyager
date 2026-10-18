//! Operator-only UUID grant using the API's database configuration. This tool
//! does not start HTTP, contact identity providers, or run migrations.
use pinvoyager_api::{Config, db};
use std::process::ExitCode;
use uuid::Uuid;

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args_os().skip(1)).await {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Accepts one UUID or a help flag and returns an operator-facing result message.
/// Grants existing accounts without changing suspension; missing users, invalid
/// arguments, configuration failures, and database failures return errors.
async fn run(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = args.into_iter().collect();
    let usage = "Usage: pinvoyager-admin UUID";
    if args.len() != 1 {
        return Err(usage.into());
    }
    if args[0] == "--help" || args[0] == "-h" {
        return Ok(usage.into());
    }
    let id = args[0]
        .to_str()
        .ok_or("UUID must be valid UTF-8")?
        .parse::<Uuid>()?;
    let config = Config::load()?;
    let pool = db::connect(&config.database).await?;
    let mut tx = pool.begin().await?;
    // Lock before reading so concurrent grants report an unchanged role correctly.
    let role: Option<bool> =
        sqlx::query_scalar("SELECT is_site_admin FROM users WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    match role {
        None => Err(format!("User {id} does not exist").into()),
        Some(true) => {
            tx.commit().await?;
            Ok(format!(
                "User {id} is already a site administrator; no changes made"
            ))
        }
        Some(false) => {
            sqlx::query("UPDATE users SET is_site_admin=true,updated_at=now() WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(format!("Site administrator privileges granted to {id}"))
        }
    }
}
