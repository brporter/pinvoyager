//! Async SQL repositories. Executor parameters allow callers to use a pool
//! or combine operations in a transaction. Connections are borrowed only for
//! database work; image processing and filesystem work belong in `photos`.

pub mod identities;
pub mod photos;
pub mod users;
pub mod voyages;
