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
        assert_eq!(g.apply(51.5, -0.12).precision_m(), g.half_cell_m());
    }
}

#[test]
fn precision_m_serializes_for_coarse_and_is_absent_for_exact() {
    // The map switches between a pin and an area on the presence of this
    // key, so its absence determines the rendered marker.
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
