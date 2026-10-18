//! Provider authentication with browser-bound attempts and server-side sessions.
//!
//! ID tokens are verified and discarded. Provider access and refresh tokens are
//! not persisted. Session and browser-binding token hashes are stored separately.
//! Accounts match on provider and subject; linking requires an authenticated account.

pub mod attempt;
/// Development sign-in bypass, compiled only when debug assertions are enabled.
#[cfg(debug_assertions)]
pub mod dev;
pub mod extract;
pub mod provider;
pub(crate) mod providers;
pub mod session;
pub(crate) mod sign_in;

pub use extract::{CurrentUser, MaybeUser};
pub use provider::{AuthProvider, ProviderKind, ProviderRegistry, VerifiedIdentity};

/// Authentication failure categories used for logging. Verification failures
/// share a client-visible response; an unconfigured provider returns 404.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("provider is not configured")]
    ProviderNotConfigured,

    #[error("no matching pending authorization attempt")]
    UnknownState,

    #[error("authorization attempt has expired")]
    AttemptExpired,

    #[error("token verification failed: {0}")]
    TokenVerification(String),

    #[error("the provider returned an error: {0}")]
    Provider(String),

    #[error("this identity is already linked to another account")]
    IdentityAlreadyLinked,

    #[error("provider discovery or configuration failed: {0}")]
    Configuration(String),
}
