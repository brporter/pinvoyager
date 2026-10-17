//! Shared application state.
//!
//! Cheap to clone — `PgPool` is internally reference-counted and the rest sits
//! behind `Arc`. Axum clones this per request, so it must stay cheap.

use std::sync::Arc;

use sqlx::PgPool;

use crate::auth::ProviderRegistry;
use crate::config::Config;
use crate::photos::worker::VariantQueue;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub upload_limiter: Arc<crate::photos::rate_limit::UploadLimiter>,
    pub config: Arc<Config>,
    /// Providers initialized at startup. Unknown signing keys can trigger
    /// rate-limited JWKS refreshes during verification.
    pub providers: Arc<ProviderRegistry>,
    /// Handle onto the variant worker. Uploads enqueue and return; encoding
    /// happens off the request path (D8).
    pub variants: VariantQueue,
}

impl AppState {
    /// Takes an `Arc<Config>` rather than a `Config` because the variant worker
    /// needs the same configuration and is constructed *before* this — the
    /// worker cannot take `AppState`, since `AppState` contains its handle.
    pub fn new(
        pool: PgPool,
        config: Arc<Config>,
        providers: ProviderRegistry,
        variants: VariantQueue,
    ) -> Self {
        Self {
            pool,
            upload_limiter: Arc::default(),
            config,
            providers: Arc::new(providers),
            variants,
        }
    }
}
