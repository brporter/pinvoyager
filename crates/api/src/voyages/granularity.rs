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
mod tests {
    use super::*;

    /// Every granularity that actually rounds. `Exact` is tested on its own,
    /// because the property it has to satisfy is the opposite one.
    const COARSE: [Granularity; 3] = [Granularity::Km1, Granularity::Km10, Granularity::Km50];

    /// The sphere implied by [`KM_PER_DEG_LAT`]. Deriving it rather than
    /// hard-coding a radius keeps the tests measuring the same planet the
    /// implementation rounds on, so a tolerance failure means the rounding is
    /// wrong and not that two constants disagree.
    const EARTH_RADIUS_KM: f64 = KM_PER_DEG_LAT * 180.0 / std::f64::consts::PI;

    fn cell_km(g: Granularity) -> f64 {
        g.cell_km().expect("coarse granularities have a cell size")
    }

    /// Great-circle distance. Haversine rather than flat-earth arithmetic
    /// because the whole point of these tests is what happens at 70° and across
    /// the antimeridian, which is exactly where flat arithmetic lies.
    fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
        let dp = p2 - p1;
        let dl = (lon2 - lon1).to_radians();
        let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);

        2.0 * EARTH_RADIUS_KM * h.sqrt().asin()
    }

    /// North-south error in kilometers.
    fn lat_error_km(truth_lat: f64, c: &Coordinate) -> f64 {
        (c.latitude() - truth_lat).abs() * KM_PER_DEG_LAT
    }

    /// East-west error in kilometers **at the true latitude** — the number a
    /// degrees-only assertion cannot see, and the one the cos correction exists
    /// to bound.
    fn lon_error_km(truth_lat: f64, truth_lon: f64, c: &Coordinate) -> f64 {
        let mut delta = (c.longitude() - truth_lon).abs();
        if delta > 180.0 {
            delta = 360.0 - delta; // the short way round
        }

        delta * KM_PER_DEG_LAT * truth_lat.to_radians().cos()
    }

    /// Measures the grid on the ground rather than trusting the arithmetic:
    /// sweeps a latitude and a longitude, watches where the output jumps, and
    /// converts the spacing between adjacent cell centers into kilometers.
    ///
    /// Returns `(height_km, width_km)`.
    fn measured_cell_km(g: Granularity, lat: f64) -> (f64, f64) {
        let cos = lat.to_radians().cos();
        let nominal_deg = cell_km(g) / KM_PER_DEG_LAT;

        // Deliberately assumes the *unscaled* cell for the longitude sweep, so
        // a missing cos correction makes the sweep cover more cells rather than
        // fewer — it can never hide the bug by stepping over it.
        let height = spacing_deg(nominal_deg / 400.0, |i| g.apply(lat + i, 12.0).latitude());
        let width = spacing_deg(nominal_deg / 400.0, |i| g.apply(lat, 12.0 + i).longitude());

        (height * KM_PER_DEG_LAT, width * KM_PER_DEG_LAT * cos)
    }

    /// Mean spacing between the distinct values `f` takes as its argument walks
    /// forward in steps of `step`.
    fn spacing_deg(step: f64, f: impl Fn(f64) -> f64) -> f64 {
        let mut centers: Vec<f64> = Vec::new();
        for i in 0..4000 {
            let value = f(i as f64 * step);
            if centers.last().is_none_or(|&last| last != value) {
                centers.push(value);
            }
        }

        assert!(
            centers.len() >= 3,
            "the sweep did not cross two cell boundaries; it cannot measure a spacing"
        );

        // Drop the first center: the sweep starts partway into its cell, so
        // only the gaps between whole cells are a spacing.
        let spanned = centers[centers.len() - 1] - centers[1];

        spanned / (centers.len() - 2) as f64
    }

    #[test]
    fn exact_passes_both_values_through_untouched() {
        // Bit-for-bit, including the values a "tidying" implementation would be
        // tempted to normalize: the antimeridian, the pole, and negative zero.
        for (lat, lon) in [
            (48.858_370_1_f64, 2.294_481_f64),
            (-33.868_820, 151.209_290),
            (0.0, 0.0),
            (-0.0, -0.0),
            (90.0, 180.0),
            (-90.0, -180.0),
            (89.999_999_9, 179.999_999_9),
        ] {
            let c = Granularity::Exact.apply(lat, lon);

            assert_eq!(c.latitude().to_bits(), lat.to_bits(), "latitude {lat}");
            assert_eq!(c.longitude().to_bits(), lon.to_bits(), "longitude {lon}");
            assert_eq!(c.precision_m(), None);
        }
    }

    #[test]
    fn rounding_is_stable_across_calls() {
        // Not a nicety. An unstable answer jitters between page loads, and —
        // far worse — lets anyone who reloads enough times average the noise
        // away and recover the position the granularity was hiding.
        for g in COARSE {
            for (lat, lon) in [
                (48.858_370, 2.294_481),
                (-33.868_820, 151.209_290),
                (69.649_2, 18.955_3),
                (0.0, 0.0),
            ] {
                let first = g.apply(lat, lon);

                for _ in 0..64 {
                    assert_eq!(g.apply(lat, lon), first, "{} at {lat},{lon}", g.as_str());
                }

                // Same granularity arrived at by a different route — this is
                // how a handler gets one, out of the voyage's stored string.
                let parsed = Granularity::parse(g.as_str()).expect("round trips");
                assert_eq!(parsed.apply(lat, lon), first);
            }
        }
    }

    #[test]
    fn rounding_an_already_rounded_point_changes_nothing() {
        // The published value must be a fixed point of the transform. If it
        // were not, a coordinate that made a second trip through the boundary
        // would drift, and drift is a signal.
        for g in COARSE {
            for (lat, lon) in [
                (48.858_370, 2.294_481),
                (69.649_2, 18.955_3),
                (-12.5, -77.1),
            ] {
                let once = g.apply(lat, lon);
                let twice = g.apply(once.latitude(), once.longitude());

                assert_eq!(twice, once, "{} at {lat},{lon}", g.as_str());
            }
        }
    }

    #[test]
    fn points_in_one_cell_collapse_to_one_point() {
        for g in COARSE {
            let anchor = g.apply(45.464_2, 9.190_0);
            let lat_cell = cell_km(g) / KM_PER_DEG_LAT;
            let lon_cell = lat_cell / longitude_cell_scale(anchor.latitude());

            // Every corner and edge of the cell, short of the boundary itself.
            for (dlat, dlon) in [
                (0.0, 0.0),
                (0.49, 0.49),
                (-0.49, -0.49),
                (0.49, -0.49),
                (-0.49, 0.49),
                (0.0, 0.30),
                (-0.30, 0.0),
            ] {
                let nearby = g.apply(
                    anchor.latitude() + dlat * lat_cell,
                    anchor.longitude() + dlon * lon_cell,
                );

                assert_eq!(
                    nearby,
                    anchor,
                    "{} should lump {dlat},{dlon} of a cell away into one point",
                    g.as_str()
                );
            }

            // And the grid has not degenerated into "everything is one point":
            // a full cell away is a different cell.
            let elsewhere = g.apply(anchor.latitude() + lat_cell, anchor.longitude() + lon_cell);
            assert_ne!(elsewhere, anchor, "{}", g.as_str());
        }
    }

    #[test]
    fn ground_error_is_bounded_by_half_a_cell_at_every_latitude() {
        for g in COARSE {
            let cell = cell_km(g);
            let lat_cell_deg = cell / KM_PER_DEG_LAT;

            for base_lat in [0.0_f64, 45.0, 60.0, 70.0, -45.0, -70.0] {
                let lon_cell_deg = lat_cell_deg / base_lat.to_radians().cos();

                // Walk a couple of cells in each axis at an irrational-ish
                // stride, so the samples land all over the cell instead of
                // repeatedly on the same offset from a boundary.
                for i in 0..17 {
                    for j in 0..17 {
                        let lat = base_lat + i as f64 * lat_cell_deg * 0.137;
                        let lon = 7.0 + j as f64 * lon_cell_deg * 0.137;
                        let c = g.apply(lat, lon);
                        let at = format!("{} at {lat},{lon}", g.as_str());

                        // Half a cell on each axis, in meters on the ground.
                        assert!(
                            lat_error_km(lat, &c) <= cell * 0.5 * 1.001,
                            "{at}: {} km north-south",
                            lat_error_km(lat, &c)
                        );
                        // The 5% headroom is the cos(true)/cos(rounded) ratio
                        // from sizing the cell off the rounded latitude; see
                        // `longitude_cell_scale`.
                        assert!(
                            lon_error_km(lat, lon, &c) <= cell * 0.5 * 1.05,
                            "{at}: {} km east-west",
                            lon_error_km(lat, lon, &c)
                        );
                        // And half a diagonal as the crow flies.
                        let d = distance_km(lat, lon, c.latitude(), c.longitude());
                        assert!(d <= cell * 0.72, "{at}: {d} km away");
                    }
                }
            }
        }
    }

    #[test]
    fn a_cell_is_as_wide_on_the_ground_as_advertised() {
        // The test that catches the missing cos correction. A fixed-degree grid
        // measures 10 km at the equator, 7.1 km at 45°, 5 km at 60° and 3.4 km
        // at 70° — the product promising less precision than it delivers, which
        // is the direction that gets someone found.
        for g in COARSE {
            let cell = cell_km(g);

            for lat in [0.0, 45.0, 60.0, 70.0, -60.0] {
                let (height, width) = measured_cell_km(g, lat);
                println!(
                    "{} at {lat:>5}°: {height:.3} km tall, {width:.3} km wide",
                    g.as_str()
                );

                assert!(
                    (height - cell).abs() < cell * 0.02,
                    "{} at {lat}°: cell is {height} km tall, want ~{cell}",
                    g.as_str()
                );
                assert!(
                    (width - cell).abs() < cell * 0.05,
                    "{} at {lat}°: cell is {width} km wide, want ~{cell}",
                    g.as_str()
                );
            }
        }
    }

    #[test]
    fn precision_m_is_half_the_cell_in_meters() {
        assert_eq!(Granularity::Km1.apply(0.0, 0.0).precision_m(), Some(500));
        assert_eq!(Granularity::Km10.apply(0.0, 0.0).precision_m(), Some(5_000));
        assert_eq!(
            Granularity::Km50.apply(0.0, 0.0).precision_m(),
            Some(25_000)
        );
        assert_eq!(Granularity::Exact.apply(0.0, 0.0).precision_m(), None);

        // The published precision and the drawn radius are one claim.
        for g in [
            Granularity::Exact,
            Granularity::Km1,
            Granularity::Km10,
            Granularity::Km50,
        ] {
            assert_eq!(g.apply(51.5, -0.12).precision_m(), g.display_radius_m());
        }
    }

    #[test]
    fn precision_m_serializes_for_coarse_and_is_absent_for_exact() {
        // Phase 7 switches between a pin and an area on the presence of this
        // key, so its absence is load-bearing rather than cosmetic.
        let coarse = serde_json::to_value(Granularity::Km10.apply(48.858_370, 2.294_481))
            .expect("Coordinate serializes");

        assert_eq!(coarse["precision_m"], serde_json::json!(5_000));
        assert!(coarse["latitude"].is_number());
        assert!(coarse["longitude"].is_number());

        let exact = serde_json::to_value(Granularity::Exact.apply(48.858_370, 2.294_481))
            .expect("Coordinate serializes");

        assert!(
            exact.get("precision_m").is_none(),
            "exact must omit the key entirely, not send null: {exact}"
        );
        assert_eq!(exact["latitude"], serde_json::json!(48.858_370));
        assert_eq!(exact["longitude"], serde_json::json!(2.294_481));
    }

    #[test]
    fn the_origin_rounds_to_the_center_of_the_cell_it_is_in() {
        // A known answer, so a change of grid — anchor, offset, or constant —
        // cannot pass silently. Every published pin in the product moves if
        // this changes.
        let c = Granularity::Km10.apply(0.0, 0.0);

        assert!((c.latitude() - 0.044_915_559).abs() < 1e-9, "{c:?}");
        assert!((c.longitude() - 0.044_915_573).abs() < 1e-9, "{c:?}");

        // Half a cell from the origin on each axis, which is the most the
        // center of a cell can ever be from a point inside it.
        assert!(lat_error_km(0.0, &c) <= 5.0 * 1.001);
        assert!(lon_error_km(0.0, 0.0, &c) <= 5.0 * 1.001);
    }

    #[test]
    fn the_grid_is_symmetric_about_the_equator_and_the_prime_meridian() {
        // A sign error is the class of bug that puts a photograph taken in
        // Paris into the Atlantic, and it survives any test written only for
        // the northeastern quadrant.
        for g in COARSE {
            for (lat, lon) in [(48.858_370, 2.294_481), (12.5, 77.1), (69.649_2, 18.955_3)] {
                let north_east = g.apply(lat, lon);
                let south_west = g.apply(-lat, -lon);

                assert!(
                    (south_west.latitude() + north_east.latitude()).abs() < 1e-9,
                    "{}: {lat} and {} are not mirrored",
                    g.as_str(),
                    -lat
                );
                assert!(
                    (south_west.longitude() + north_east.longitude()).abs() < 1e-9,
                    "{}: {lon} and {} are not mirrored",
                    g.as_str(),
                    -lon
                );
            }
        }
    }

    #[test]
    fn coordinates_stay_in_their_own_quadrant() {
        // The blunt version of the same check: four cities, four sign
        // combinations, each still within a cell of where it started.
        for g in COARSE {
            for (name, lat, lon) in [
                ("Paris", 48.858_370, 2.294_481),
                ("Sydney", -33.868_820, 151.209_290),
                ("Lima", -12.046_4, -77.042_8),
                ("Anchorage", 61.218_1, -149.900_3),
            ] {
                let c = g.apply(lat, lon);

                assert_eq!(
                    c.latitude().is_sign_negative(),
                    lat.is_sign_negative(),
                    "{name} changed hemisphere under {}: {c:?}",
                    g.as_str()
                );
                assert_eq!(
                    c.longitude().is_sign_negative(),
                    lon.is_sign_negative(),
                    "{name} crossed the prime meridian under {}: {c:?}",
                    g.as_str()
                );
                assert!(
                    distance_km(lat, lon, c.latitude(), c.longitude()) <= cell_km(g) * 0.72,
                    "{name} moved too far under {}",
                    g.as_str()
                );
            }
        }
    }

    #[test]
    fn the_antimeridian_still_produces_a_longitude() {
        // The cell straddling ±180 has its center past it, so without wrapping
        // this emits 180.03 — a value that is not a longitude and that a map
        // will place off the edge of the world.
        for g in COARSE {
            for lon in [179.9, 179.999, 180.0, -180.0, -179.999, -179.9, -177.3] {
                for lat in [0.0, 45.0, -45.0, 71.0] {
                    let c = g.apply(lat, lon);
                    let at = format!("{} at {lat},{lon}", g.as_str());

                    assert!(
                        (-180.0..=180.0).contains(&c.longitude()),
                        "{at}: {} is not a longitude",
                        c.longitude()
                    );
                    assert!((-90.0..=90.0).contains(&c.latitude()), "{at}");

                    // Wrapping must move the point across the seam, not around
                    // the planet.
                    let d = distance_km(lat, lon, c.latitude(), c.longitude());
                    assert!(d <= cell_km(g) * 0.72, "{at}: {d} km away");
                }
            }
        }

        // Either side of the seam is one place, and the grid should treat it as
        // one place rather than as two cells 360° apart.
        let west = Granularity::Km50.apply(0.0, 179.99);
        let east = Granularity::Km50.apply(0.0, -179.99);
        assert!(
            distance_km(
                west.latitude(),
                west.longitude(),
                east.latitude(),
                east.longitude()
            ) <= 50.0,
            "{west:?} and {east:?} are 2 km apart on the ground"
        );
    }

    #[test]
    fn the_poles_do_not_blow_the_cell_up() {
        for g in COARSE {
            let cell = cell_km(g);
            // What the clamp guarantees: the longitude cell never grows past
            // the width it has at MAX_SCALED_LATITUDE_DEG.
            let widest_deg = cell / KM_PER_DEG_LAT / MAX_SCALED_LATITUDE_DEG.to_radians().cos();

            for lat in [90.0, -90.0, 89.999_9, -89.999_9, 89.0, 86.5, 85.0] {
                for lon in [0.0, 42.0, -173.0, 179.99] {
                    let c = g.apply(lat, lon);
                    let at = format!("{} at {lat},{lon}", g.as_str());

                    assert!(
                        c.latitude().is_finite() && c.longitude().is_finite(),
                        "{at}"
                    );
                    assert!(
                        (-90.0..=90.0).contains(&c.latitude()),
                        "{at}: {} is not a latitude",
                        c.latitude()
                    );
                    assert!(
                        (-180.0..=180.0).contains(&c.longitude()),
                        "{at}: {} is not a longitude",
                        c.longitude()
                    );

                    // Unclamped, cos(90°) is 6e-17 and the cell is 10^15
                    // degrees wide: the reported longitude would be arbitrary.
                    let mut moved = (c.longitude() - lon).abs();
                    if moved > 180.0 {
                        moved = 360.0 - moved;
                    }
                    assert!(
                        moved <= widest_deg / 2.0 + 1e-9,
                        "{at}: longitude moved {moved}°, more than the clamped half-cell \
                         of {}°",
                        widest_deg / 2.0
                    );

                    // And on the ground, where the meridians have converged,
                    // the error is dominated by the latitude axis as ever.
                    let d = distance_km(lat, lon, c.latitude(), c.longitude());
                    assert!(d <= cell * 0.72, "{at}: {d} km away");
                }
            }
        }
    }

    #[test]
    fn coarse_actually_moves_a_typical_coordinate() {
        // The guard against an implementation that satisfies every property
        // above by returning its input.
        let lat = 48.858_370_1;
        let lon = 2.294_481_3;

        for g in COARSE {
            let c = g.apply(lat, lon);

            assert_ne!(c.latitude(), lat, "{} left the latitude alone", g.as_str());
            assert_ne!(
                c.longitude(),
                lon,
                "{} left the longitude alone",
                g.as_str()
            );

            // T5 in the plan is a grep of the response body for the stored
            // value, so the rounded value must not merely differ numerically —
            // it must not contain the exact one as a prefix either.
            let rendered = serde_json::to_string(&c).expect("serializes");
            assert!(!rendered.contains("48.8583701"), "{rendered}");
            assert!(!rendered.contains("2.2944813"), "{rendered}");
        }
    }

    #[test]
    fn coarser_granularity_is_never_more_precise() {
        let lat = 48.858_370;
        let lon = 2.294_481;

        let km1 = Granularity::Km1.apply(lat, lon);
        let km10 = Granularity::Km10.apply(lat, lon);
        let km50 = Granularity::Km50.apply(lat, lon);

        assert!(distance_km(lat, lon, km1.latitude(), km1.longitude()) <= 1.0);
        assert!(distance_km(lat, lon, km10.latitude(), km10.longitude()) <= 10.0);
        assert!(distance_km(lat, lon, km50.latitude(), km50.longitude()) <= 50.0);

        assert!(km1.precision_m() < km10.precision_m());
        assert!(km10.precision_m() < km50.precision_m());
    }
}
