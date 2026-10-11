//! PinVoyager API.
//!
//! Module layout is deliberately flat and boring. The invariants worth knowing
//! before changing anything here:
//!
//! * **D1** — no credential of any kind is ever stored. There is no password
//!   column, no hash, no persisted provider token. If a change requires one,
//!   the change is wrong.
//! * **D8** — every database operation is async, and nothing that blocks may
//!   run on a runtime worker thread. CPU-bound work goes through
//!   [`blocking::run_blocking`], never inline in a handler.
//! * **D7** — user uploads are served only through an authorizing handler,
//!   never as static files.

pub mod auth;
pub mod blocking;
pub mod config;
pub mod db;
pub mod error;
pub mod http;
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
