//! Voyages owned by one account, with draft/public state and a location precision.
//!
//! Public detail and map responses apply the selected granularity. Owner-only
//! photo metadata retains exact coordinates. Draft reads return 404 to anyone
//! other than the owner. See [`granularity`] for coordinate rounding.

pub mod granularity;

pub use granularity::{Coordinate, Granularity};
