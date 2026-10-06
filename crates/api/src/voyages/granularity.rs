//! Location granularity — the privacy boundary.
//!
//! CONTRACT — `Granularity`, `Coordinate` and their signatures are fixed;
//! the rounding implementation is owned by the granularity agent.
//!
//! # Why this module exists at all
//!
//! Every voyage is public (D6). Phase 4 deliberately stores **exact**
//! coordinates so that a user can loosen or tighten precision later, which
//! means the only thing standing between an EXIF coordinate and the world is
//! this code.
//!
//! Enforcing that per endpoint guarantees that the one endpoint somebody adds
//! at six o'clock on a Friday becomes the leak. So it is enforced by the type
//! system instead: [`Coordinate`] has **private fields** and exactly one
//! constructor, which requires a [`Granularity`]. There is no way to put a
//! latitude into a response body without having stated a precision, and Rust's
//! module privacy makes that unforgeable rather than conventional.
//!
//! The row type that carries raw coordinates — `db::repo::photos::Photo` —
//! deliberately does not implement `Serialize`. Returning one from a handler is
//! a compile error, not a review comment.
//!
//! # The rounding
//!
//! Grid rounding to the **center** of a cell, which gives three properties the
//! plan asks for:
//!
//! * **Stable.** The same input always yields the same output. Re-sampling per
//!   request would make pins jitter between page loads and, far worse, would
//!   let repeated observation average the noise away and recover the true
//!   position.
//! * **Clustering.** Photos that fall in one cell collapse to one point, which
//!   is the "lump them together" behavior the plan specifies — and it is also
//!   what stops a coarse voyage from displaying a suspiciously even scatter of
//!   distinct pins.
//! * **Bounded error.** The center is at most half a cell from the truth on
//!   each axis, rather than up to a full cell as flooring would give.
//!
//! ## Longitude is not latitude
//!
//! A degree of latitude is ~111.32 km everywhere. A degree of longitude is
//! 111.32 km at the equator and shrinks with `cos(latitude)` — about 55 km at
//! 60°N, and near zero at the poles.
//!
//! So a fixed-degree grid would produce cells that are *narrower* than
//! advertised at high latitudes: a "~10 km" voyage in northern Norway would
//! actually be pinned to about 5 km. That is the dangerous direction to be
//! wrong in — the product would be promising less precision than it delivers.
//!
//! The longitude cell is therefore divided by `cos(latitude)` so its ground
//! width stays close to the target. It is computed from the **rounded**
//! latitude, not the raw one, so the cell a point lands in does not depend on
//! the very value being hidden.
//!
//! ## Near the poles
//!
//! `cos` runs to zero at the pole, so the correction runs to infinity: a
//! literal `cell_km / cos(89.999°)` is a cell tens of thousands of degrees
//! wide, which is not a grid so much as a floating-point accident. The scaling
//! is therefore frozen at 85° — see `longitude_cell_scale` for why that
//! latitude, and why stopping there is safe in the direction that matters.

use serde::Serialize;

/// Kilometers per degree of latitude.
///
/// Constant to about half a percent between the equator and the pole on the
/// real spheroid, which is two orders of magnitude finer than the smallest cell
/// this module offers, so a single spherical figure is used rather than a
/// geodetic one.
const KM_PER_DEG_LAT: f64 = 111.32;

/// The latitude past which the longitude cell stops being widened.
///
/// See [`longitude_cell_scale`]. 85° is chosen rather than something closer to
/// the pole because the widening factor there is already 11.5×, and because
/// there is nothing above it to protect: the northernmost land on Earth is
/// around 83°N and the northernmost inhabited place around 82.5°N.
const MAX_SCALED_LATITUDE_DEG: f64 = 85.0;

/// The four precision steps. Values match the migration's CHECK constraint.
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

    /// Radius to draw on the map, in meters, or `None` for exact.
    ///
    /// Phase 7 renders an **area** rather than a pin whenever this is `Some`.
    /// A sharp pin at the center of a 10 km cell makes a precision claim the
    /// data cannot support, and visually undoes the choice the user made.
    pub fn display_radius_m(self) -> Option<u32> {
        self.cell_km().map(|km| (km * 500.0) as u32)
    }

    /// **The only way to build a [`Coordinate`].**
    ///
    /// Takes the exact values and returns what the world is allowed to see.
    pub fn apply(self, latitude: f64, longitude: f64) -> Coordinate {
        let Some(cell_km) = self.cell_km() else {
            // Exact means exact: no rounding, no clamping, no wrapping. Any
            // "tidying" here would be a second, undeclared transformation of
            // the one value the user asked us not to touch.
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
            // The same number as the radius phase 7 draws, deliberately: the
            // circle on the map is the precision claim, and two independently
            // computed versions of one claim would eventually disagree.
            precision_m: self.display_radius_m(),
        }
    }
}

/// The center of the `cell`-wide grid cell containing `value`, on a grid
/// anchored at zero.
///
/// The center rather than the lower edge: both collapse a cell to one point,
/// which is the clustering the plan asks for, but nothing is further than half
/// a cell from the center where a point just short of the upper edge is a whole
/// cell from the lower one.
///
/// Anchoring at zero makes the grid the same for every user and every request —
/// there is no per-voyage or per-photo offset to average away — and makes it
/// symmetric about the equator and the prime meridian: `floor` rounds toward
/// negative infinity, so `-x` lands exactly opposite `x` instead of the
/// southern and western hemispheres getting a differently shaped grid.
fn cell_center(value: f64, cell: f64) -> f64 {
    (value / cell).floor() * cell + cell / 2.0
}

/// `cos(latitude)`, floored at [`MAX_SCALED_LATITUDE_DEG`].
///
/// Dividing the longitude cell by this is what keeps its *ground* width near
/// the advertised size instead of shrinking with the meridians. Without the
/// floor the factor is unbounded, and a cell wider than the globe is both
/// meaningless and numerically hopeless.
///
/// Freezing the scaling above 85° means cells there are narrower on the ground
/// than advertised — normally the dangerous direction — and it is nonetheless
/// the right trade:
///
/// * The latitude axis is untouched, so the reported point is still up to half
///   a cell from the truth north-south. Total displacement, the thing privacy
///   actually depends on, is unchanged.
/// * East-west ground distance is what vanishes at the pole in the first place.
///   At 89° the frozen `Km50` cell is still 5.2° of longitude wide — a third of
///   a time zone — and that is 10 km of ground. Buying the missing 40 km would
///   take a cell 26° wide, which moves the pin a sixth of the way round the
///   Arctic for ambiguity the latitude axis is already supplying.
///
/// The floor also bounds a subtler error. The cell width is set by the rounded
/// latitude while the point sits at the true one, so its real ground width is
/// off by `cos(true) / cos(rounded)`. For the widest cell that ratio is 1.007 at
/// 60°, 1.011 at 70° and 1.045 at 85°, and it is unbounded as the pole is
/// approached; the clamp is what stops it running away.
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

/// A coordinate that has passed through the granularity boundary.
///
/// Private fields, no public constructor, no `From<f64>`. The only way to
/// obtain one is [`Granularity::apply`], so a latitude cannot reach a response
/// body without a precision having been chosen for it.
///
/// `precision_m` travels with the value on purpose: showing a rounded
/// coordinate without saying it is rounded invites the reader to treat it as
/// exact, which is the misreading the whole feature exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coordinate {
    latitude: f64,
    longitude: f64,
    /// Half the cell size — the maximum error on either axis. `None` when
    /// exact.
    ///
    /// Per axis, not as the crow flies: a point in the corner of its cell is
    /// half a *diagonal* from the center, about 1.41 × this. Publishing the
    /// smaller figure is the conservative choice — the truth is at least this
    /// far away and sometimes further — and it is the radius phase 7 draws,
    /// which is then the circle inscribed in the cell rather than one bulging
    /// out of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    precision_m: Option<u32>,
}

impl Coordinate {
    /// Read access for the map endpoint, which needs the numbers rather than
    /// the serialized form. Still only obtainable from an already-rounded
    /// value, so this is not a way around the boundary.
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
