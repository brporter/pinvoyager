//! Shared application state.
//!
//! Cheap to clone — `PgPool` is internally reference-counted and `Config` sits
//! behind an `Arc`. Axum clones this per request, so it must stay cheap.

use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self {
            pool,
            config: Arc::new(config),
        }
    }
}
