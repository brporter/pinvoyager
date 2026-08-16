//! Repository layer.
//!
//! Every function is `async fn` taking `impl sqlx::PgExecutor<'_>`. No function
//! here acquires a connection and holds it across unrelated work (D8).
//!
//! Taking an executor rather than a `&PgPool` is what makes both of the things
//! phase 2 needs possible: a test can pass a `&mut PgTransaction` and roll it
//! back instead of cleaning up, and phase 3 can create a user and their first
//! identity in one transaction without a parallel set of functions that take
//! one. It also means no call site can acquire a connection and forget to
//! release it — the executor is borrowed for the duration of the call and no
//! longer.
//!
//! Phase 2 needs only enough to prove the schema and the pattern. Phase 3 adds
//! the identity lookups it actually needs, against real requirements.

pub mod identities;
pub mod photos;
pub mod users;
pub mod voyages;
