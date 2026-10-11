//! Voyages.
//!
//! A voyage is a set of photographs on a map, owned by one user, published or
//! draft, with one location granularity governing every coordinate it emits.
//!
//! # The one rule
//!
//! **Exact coordinates never leave this service for a voyage set to anything
//! coarser than exact** — not in a detail response, not in a list, not in an
//! embedded object, not in an error message.
//!
//! That is enforced structurally rather than by discipline. See
//! [`granularity`]: the only type that can carry a latitude into a response has
//! private fields and one constructor, and that constructor demands a
//! precision. The row type holding raw coordinates does not implement
//! `Serialize` at all, so returning one from a handler does not compile.
//!
//! # Draft and published
//!
//! A draft is visible only to its owner and returns **404** to everyone else —
//! not 403. "Forbidden" would confirm that a voyage exists at that address,
//! which lets unpublished work be enumerated by probing ids.

pub mod granularity;

pub use granularity::{Coordinate, Granularity};
