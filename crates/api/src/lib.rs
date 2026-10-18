//! PinVoyager API.
//!
//! Provider identities and server-side sessions handle authentication without
//! password storage or persisted provider access/refresh tokens. Database work
//! is async; CPU-heavy image work and blocking file operations use the blocking
//! pool. Image handlers authorize requests and serve only re-encoded variants.

pub mod auth;
pub mod blocking;
pub mod config;
pub mod db;
pub mod error;
pub mod http;
pub mod initial_admin;
pub mod photos;
pub mod state;
pub mod telemetry;
pub mod voyages;

pub use config::Config;
pub use error::{AppError, AppResult};
pub use state::AppState;

#[cfg(test)]
#[path = "../tests/support/photo_voyage.rs"]
mod photo_voyage;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
