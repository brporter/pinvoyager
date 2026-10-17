//! Location granularity — the privacy boundary.
//!
//! Published voyages expose rounded locations (D6). The database preserves
//! exact coordinates for owners, so changing precision does not lose source data.
//!
//! [`Coordinate`] has private fields and is constructed by
//! [`Granularity::apply`]. Public response types use it to record the selected
//! precision. Repository rows do not implement `Serialize`, but handlers can
//! still copy raw numbers into other response types. Review new public
//! endpoints and test their coordinates; this type does not enforce the
//! privacy policy across every possible response.
//!
//! # The rounding
//!
//! Rounds coordinates to fixed cell centers. Identical inputs produce identical
//! outputs, photos in the same cell share a position, and each axis moves by
//! at most half its cell width before longitude wrapping.

use serde::Serialize;

/// Approximate kilometers per degree of latitude used by the spherical grid.
const KM_PER_DEG_LAT: f64 = 111.32;

/// The latitude past which the longitude cell stops being widened.
///
/// Caps the widening factor at approximately 11.5. Beyond ±85°, east-west
/// cells become narrower on the ground than the selected kilometer size.
const MAX_SCALED_LATITUDE_DEG: f64 = 85.0;

/// Supported public location precision settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    /// As recorded. The coordinate leaves the server unchanged.
    Exact,
    Km1,
    Km10,
    Km50,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::Exact => "exact",
            Granularity::Km1 => "km1",
            Granularity::Km10 => "km10",
            Granularity::Km50 => "km50",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exact" => Some(Granularity::Exact),
            "km1" => Some(Granularity::Km1),
            "km10" => Some(Granularity::Km10),
            "km50" => Some(Granularity::Km50),
            _ => None,
        }
    }

    /// Target cell size in kilometers. `None` for [`Granularity::Exact`].
    pub fn cell_km(self) -> Option<f64> {
        match self {
            Granularity::Exact => None,
            Granularity::Km1 => Some(1.0),
            Granularity::Km10 => Some(10.0),
            Granularity::Km50 => Some(50.0),
        }
    }

    /// Per-axis half-width of the rounding cell, in meters; `None` for exact.
    /// The frontend multiplies this by sqrt(2) for a circle enclosing the cell.
    pub fn half_cell_m(self) -> Option<u32> {
        self.cell_km().map(|km| (km * 500.0) as u32)
    }

    /// Constructs a coordinate at the selected precision. Exact preserves input;
    /// coarse settings round to a cell center, clamp latitude, and wrap longitude.
    /// Inputs are expected to have been validated; this method returns no error.
    pub fn apply(self, latitude: f64, longitude: f64) -> Coordinate {
        let Some(cell_km) = self.cell_km() else {
            // Preserve exact coordinates without rounding, clamping, or wrapping.
            return Coordinate {
                latitude,
                longitude,
                precision_m: None,
            };
        };

        let lat_cell_deg = cell_km / KM_PER_DEG_LAT;

        // Clamped because a cell straddling the pole rounds to past ±90, which
        // is not a latitude. The clamp only ever moves the point closer to the
        // truth, so it cannot widen the error bound.
        let lat = cell_center(latitude, lat_cell_deg).clamp(-90.0, 90.0);

        // From the rounded latitude, never the raw one. The width of the
        // longitude cell decides which cell a longitude falls into, so deriving
        // it from the exact latitude would make the published longitude a
        // function of the very value being hidden — and, just as bad, would
        // stop two photos in one cell from agreeing on an answer.
        let lon_cell_deg = lat_cell_deg / longitude_cell_scale(lat);
        let lon = wrap_longitude(cell_center(longitude, lon_cell_deg));

        Coordinate {
            latitude: lat,
            longitude: lon,
            // Per-axis half-width; the frontend draws the cell diagonal.
            precision_m: self.half_cell_m(),
        }
    }
}

/// Returns the center of the `cell`-wide interval containing `value`.
/// The grid is anchored at zero and uses floor division for negative values.
fn cell_center(value: f64, cell: f64) -> f64 {
    (value / cell).floor() * cell + cell / 2.0
}

/// Returns the cosine of absolute latitude, clamping latitude to 85° first.
/// Longitude cell widths divide by this value. Beyond the cap, east-west
/// ground widths shrink toward the poles; the nominal width is approximate.
fn longitude_cell_scale(latitude_deg: f64) -> f64 {
    latitude_deg
        .abs()
        .min(MAX_SCALED_LATITUDE_DEG)
        .to_radians()
        .cos()
}

/// Normalizes to `[-180, 180)`, or to 180.0 itself in the one floating-point
/// case where `rem_euclid` rounds up to its modulus. Both are the same meridian
/// and both are values a map will accept.
///
/// The grid does not divide the globe evenly, so the cell containing a point
/// just west of the antimeridian has its center just east of it — 180.03, which
/// is *not* a value a map will accept. Wrapping is a rotation of the sphere, so
/// it changes no distance and loses no precision.
fn wrap_longitude(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

/// Coordinates produced by [`Granularity::apply`], with the nominal precision.
/// Private fields keep construction within this module; other response types
/// still need to apply granularity before exposing public coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coordinate {
    latitude: f64,
    longitude: f64,
    /// Nominal per-axis half-width in meters, or `None` for exact coordinates.
    /// A cell corner is approximately sqrt(2) times this distance from its center.
    /// Longitude widths vary with latitude, especially beyond the scaling cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    precision_m: Option<u32>,
}

impl Coordinate {
    /// Returns the latitude after applying the selected precision.
    pub fn latitude(&self) -> f64 {
        self.latitude
    }

    pub fn longitude(&self) -> f64 {
        self.longitude
    }

    pub fn precision_m(&self) -> Option<u32> {
        self.precision_m
    }
}

#[cfg(test)]
#[path = "granularity_tests.rs"]
mod tests;
