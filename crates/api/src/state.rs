//! Shared application state.
//!
//! Cheap to clone — `PgPool` is internally reference-counted and the rest sits
//! behind `Arc`. Axum clones this per request, so it must stay cheap.

use std::sync::Arc;

use sqlx::PgPool;

use crate::auth::ProviderRegistry;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    /// Built once at startup: OIDC discovery and JWKS are fetched here, not per
    /// sign-in. A discovery round trip on every authorization would add latency
    /// to every sign-in and put avoidable load on the provider.
    pub providers: Arc<ProviderRegistry>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config, providers: ProviderRegistry) -> Self {
        Self {
            pool,
            config: Arc::new(config),
            providers: Arc::new(providers),
        }
    }
}
