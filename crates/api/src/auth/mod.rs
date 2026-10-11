//! Authentication.
//!
//! The whole of this module operates under D1: **no credential of any kind is
//! stored.** No password, no hash of one, no provider access or refresh token.
//! A provider's ID token is verified on receipt and discarded; what persists is
//! only the fact that a provider vouched for a subject, plus a session we
//! issued ourselves.
//!
//! ## The shape of the flow
//!
//! ```text
//! GET  /auth/{provider}/start     create attempt (state, nonce, PKCE) → 302
//! GET  /auth/{provider}/callback  consume attempt → verify → session cookie
//! POST /auth/apple/callback       same, but Apple form-POSTs
//! ```
//!
//! ## Two rules that are not negotiable
//!
//! **Never match accounts on email** (D3). Two providers can return the same
//! address for the same human, and not every provider verifies address
//! ownership, so auto-merging on email is an account-takeover vector. Identity
//! matching is on `(provider, subject)` and nothing else. This service does not
//! store email at all, which removes the temptation structurally.
//!
//! **Linking flows through the already-authenticated user.** The user proves
//! control of account A, *then* adds provider B to it. The reverse — proving B
//! and claiming A — is exactly what makes email-based merging exploitable.

pub mod attempt;
/// Forged sign-in for development. Absent from release builds entirely — see
/// the module docs before assuming it is merely disabled there.
#[cfg(debug_assertions)]
pub mod dev;
pub mod extract;
pub mod provider;
pub(crate) mod providers;
pub mod session;

pub use extract::{CurrentUser, MaybeUser};
pub use provider::{AuthProvider, ProviderKind, ProviderRegistry, VerifiedIdentity};

/// Errors specific to the authentication flow.
///
/// These deliberately do **not** distinguish failure modes to the client. A
/// rejected `state`, a bad signature, an expired token and a mismatched nonce
/// all present identically; telling them apart for a user would also tell them
/// apart for someone probing the endpoint. The variant exists so the *log* can
/// say which one it was.
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
