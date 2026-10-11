use super::*;

use crate::photos::decode::testutil::{ScratchDir, exiftool, write_jpeg};

/// Tolerance for a coordinate that has been through exiftool's
/// decimal → DMS-rational → decimal round trip. Roughly a centimeter;
/// tight enough that any *rounding* of the value would blow straight
/// through it.
const EPSILON: f64 = 1e-9;

#[test]
fn capture_time_fallbacks_offsets_and_invalid_fields_are_handled() {
    use ::exif::{Rational, Tag, Value};

    assert_eq!(
        offset_time_tag(Tag::DateTimeOriginal),
        Tag::OffsetTimeOriginal
    );
    assert_eq!(
        offset_time_tag(Tag::DateTimeDigitized),
        Tag::OffsetTimeDigitized
    );
    assert_eq!(offset_time_tag(Tag::DateTime), Tag::OffsetTime);

    let capture = Value::Ascii(vec![b"2021:07:04 12:34:56".to_vec()]);
    let offset = Value::Ascii(vec![b"+02:00".to_vec()]);
    assert_eq!(
        captured_at_values(&capture, Some(&offset))
            .expect("valid test fixture")
            .to_rfc3339(),
        "2021-07-04T10:34:56+00:00"
    );
    assert!(captured_at_values(&Value::Short(vec![1]), None).is_none());
    assert!(
        captured_at_values(&Value::Ascii(vec![b"2021:02:31 12:34:56".to_vec()]), None,).is_none()
    );
    assert!(
        captured_at_values(&Value::Ascii(vec![b"2021:07:04 25:34:56".to_vec()]), None,).is_none()
    );

    let short_dms = Value::Rational(vec![Rational::from((1, 1)), Rational::from((2, 1))]);
    let valid_dms = Value::Rational(vec![
        Rational::from((48, 1)),
        Rational::from((51, 1)),
        Rational::from((29, 1)),
    ]);
    let infinite_dms = Value::Rational(vec![
        Rational::from((1, 0)),
        Rational::from((0, 1)),
        Rational::from((0, 1)),
    ]);
    let north = Value::Ascii(vec![b"N".to_vec()]);
    assert!(coordinate_values(&Value::Short(vec![1]), &north, Tag::GPSLatitude).is_none());
    assert!(coordinate_values(&short_dms, &north, Tag::GPSLatitude).is_none());
    assert!(coordinate_values(&infinite_dms, &north, Tag::GPSLatitude).is_none());
    assert!(coordinate_values(&valid_dms, &Value::Short(vec![1]), Tag::GPSLatitude).is_none());
    assert!(
        coordinate_values(
            &valid_dms,
            &Value::Ascii(vec![b"X".to_vec()]),
            Tag::GPSLatitude,
        )
        .is_none()
    );
}

/// The four hemispheres, because a single northern-eastern test case
/// passes just as happily with the sign logic deleted entirely.
///
/// Each is written as degrees/minutes/seconds plus an explicit reference,
/// which is how a camera actually stores it: the magnitude is unsigned and
/// the hemisphere lives in its own tag.
#[test]
fn gps_signs_follow_the_reference_in_every_hemisphere() {
    let dir = ScratchDir::new("gps-hemispheres");

    // (label, lat DMS, lat ref, lon DMS, lon ref, expected lat, expected lon)
    let cases = [
        // Kyoto — the easy quadrant, both positive.
        (
            "kyoto",
            "35 0 36.0",
            "N",
            "135 46 6.0",
            "E",
            35.01,
            135.7683333333333,
        ),
        // Sydney — southern, so the latitude must go negative.
        (
            "sydney",
            "33 51 54.0",
            "S",
            "151 12 36.0",
            "E",
            -33.865,
            151.21,
        ),
        // New York — western, so the longitude must go negative. Get this
        // one wrong and the photo appears in Kazakhstan.
        (
            "new-york",
            "40 42 46.0",
            "N",
            "74 0 21.6",
            "W",
            40.71277777777778,
            -74.006,
        ),
        // Buenos Aires — both negative, the case that catches a sign
        // applied to only one of the two coordinates.
        (
            "buenos-aires",
            "34 36 12.0",
            "S",
            "58 22 54.0",
            "W",
            -34.60333333333333,
            -58.38166666666667,
        ),
    ];

    for (label, lat, lat_ref, lon, lon_ref, want_lat, want_lon) in cases {
        let path = write_jpeg(&dir, label, 8, 8);
        exiftool(&[
            "-overwrite_original",
            &format!("-GPSLatitude={lat}"),
            &format!("-GPSLatitudeRef={lat_ref}"),
            &format!("-GPSLongitude={lon}"),
            &format!("-GPSLongitudeRef={lon_ref}"),
            path.to_str().expect("scratch path is utf-8"),
        ]);

        let data = extract(&path);
        let got_lat = data.latitude.expect("latitude present");
        let got_lon = data.longitude.expect("longitude present");

        assert!(
            (got_lat - want_lat).abs() < EPSILON,
            "{label}: latitude {got_lat} != {want_lat}"
        );
        assert!(
            (got_lon - want_lon).abs() < EPSILON,
            "{label}: longitude {got_lon} != {want_lon}"
        );
        assert_eq!(
            got_lat.is_sign_negative(),
            lat_ref == "S",
            "{label}: latitude sign must come from GPSLatitudeRef"
        );
        assert_eq!(
            got_lon.is_sign_negative(),
            lon_ref == "W",
            "{label}: longitude sign must come from GPSLongitudeRef"
        );
    }
}

/// D6 requires exact coordinates in owner metadata:
/// granularity is applied when serving, so any rounding here is a precision
/// loss that cannot be undone and that no later setting can recover.
#[test]
fn coordinates_are_not_rounded() {
    let dir = ScratchDir::new("gps-exact");
    let path = write_jpeg(&dir, "precise", 8, 8);

    // Deliberately not a round number of seconds.
    exiftool(&[
        "-overwrite_original",
        "-GPSLatitude=51 30 26.3472",
        "-GPSLatitudeRef=N",
        "-GPSLongitude=0 7 39.9312",
        "-GPSLongitudeRef=W",
        path.to_str().expect("scratch path is utf-8"),
    ]);

    let data = extract(&path);
    let latitude = data.latitude.expect("latitude present");
    let longitude = data.longitude.expect("longitude present");

    let want_lat = 51.0 + 30.0 / 60.0 + 26.3472 / 3600.0;
    let want_lon = -(7.0 / 60.0 + 39.9312 / 3600.0);
    assert!((latitude - want_lat).abs() < EPSILON, "got {latitude}");
    assert!((longitude - want_lon).abs() < EPSILON, "got {longitude}");

    // And the value carries more than the ~3 decimal places a "round it at
    // ingest" implementation would leave behind.
    assert_ne!(latitude, (latitude * 1_000.0).round() / 1_000.0);
}

#[test]
fn a_capture_time_without_a_zone_is_read_as_utc() {
    let dir = ScratchDir::new("exif-time");
    let path = write_jpeg(&dir, "timed", 8, 8);
    exiftool(&[
        "-overwrite_original",
        "-DateTimeOriginal=2021:07:04 12:34:56",
        path.to_str().expect("scratch path is utf-8"),
    ]);

    let captured = extract(&path).captured_at.expect("capture time present");
    assert_eq!(captured.to_rfc3339(), "2021-07-04T12:34:56+00:00");
}

#[test]
fn a_capture_time_with_an_offset_is_converted_to_utc() {
    let dir = ScratchDir::new("exif-time-offset");
    let path = write_jpeg(&dir, "timed", 8, 8);
    exiftool(&[
        "-overwrite_original",
        "-DateTimeOriginal=2021:07:04 12:34:56",
        "-OffsetTimeOriginal=+02:00",
        path.to_str().expect("scratch path is utf-8"),
    ]);

    let captured = extract(&path).captured_at.expect("capture time present");
    assert_eq!(captured.to_rfc3339(), "2021-07-04T10:34:56+00:00");
}

#[test]
fn orientation_is_read_and_range_checked() {
    let dir = ScratchDir::new("exif-orientation");

    for value in 1..=8u16 {
        let path = write_jpeg(&dir, &format!("o{value}"), 8, 8);
        exiftool(&[
            "-overwrite_original",
            &format!("-Orientation#={value}"),
            path.to_str().expect("scratch path is utf-8"),
        ]);
        assert_eq!(extract(&path).orientation, Some(value));
    }

    // 0 is a common value from careless writers and is not a valid
    // orientation. It must not be reported as one.
    let path = write_jpeg(&dir, "zero", 8, 8);
    exiftool(&[
        "-overwrite_original",
        "-Orientation#=0",
        path.to_str().expect("scratch path is utf-8"),
    ]);
    assert_eq!(extract(&path).orientation, None);
}

/// The normal case, not the exceptional one: screenshots, scans, and
/// anything shot with location services off.
#[test]
fn a_file_without_exif_yields_defaults_rather_than_an_error() {
    let dir = ScratchDir::new("exif-absent");
    let path = write_jpeg(&dir, "bare", 16, 16);

    let data = extract(&path);
    assert!(data.captured_at.is_none());
    assert!(data.latitude.is_none());
    assert!(data.longitude.is_none());
    assert!(data.orientation.is_none());
}

#[test]
fn a_malformed_file_yields_defaults_rather_than_a_panic() {
    let dir = ScratchDir::new("exif-malformed");

    // Not an image at all.
    let garbage = dir.path().join("garbage.bin");
    std::fs::write(
        &garbage,
        b"\xff\xd8\xff\xe1\x00\x20Exif\0\0 truncated nonsense",
    )
    .expect("write garbage");
    assert!(extract(&garbage).captured_at.is_none());

    // Not a file at all.
    let missing = dir.path().join("does-not-exist.jpg");
    assert!(extract(&missing).latitude.is_none());
}

/// A photo can carry a location and no clock, or a clock and no location.
/// Neither absence may take the other field down with it.
#[test]
fn fields_are_independent() {
    let dir = ScratchDir::new("exif-partial");

    let gps_only = write_jpeg(&dir, "gps-only", 8, 8);
    exiftool(&[
        "-overwrite_original",
        "-GPSLatitude=48 51 29.6",
        "-GPSLatitudeRef=N",
        "-GPSLongitude=2 17 40.2",
        "-GPSLongitudeRef=E",
        gps_only.to_str().expect("scratch path is utf-8"),
    ]);
    let data = extract(&gps_only);
    assert!(data.latitude.is_some() && data.longitude.is_some());
    assert!(data.captured_at.is_none());

    let time_only = write_jpeg(&dir, "time-only", 8, 8);
    exiftool(&[
        "-overwrite_original",
        "-DateTimeOriginal=1999:12:31 23:59:59",
        time_only.to_str().expect("scratch path is utf-8"),
    ]);
    let data = extract(&time_only);
    assert!(data.captured_at.is_some());
    assert!(data.latitude.is_none() && data.longitude.is_none());
}

/// A latitude tag with no reference tag cannot be signed, and a coordinate
/// we cannot sign is worse than none: it would place the photo confidently
/// in the wrong hemisphere.
#[test]
fn a_coordinate_without_its_reference_is_dropped() {
    let dir = ScratchDir::new("exif-no-ref");
    let path = write_jpeg(&dir, "unsigned", 8, 8);

    exiftool(&[
        "-overwrite_original",
        "-GPSLatitude=48 51 29.6",
        "-GPSLatitudeRef=N",
        "-GPSLongitude=2 17 40.2",
        "-GPSLongitudeRef=E",
        path.to_str().expect("scratch path is utf-8"),
    ]);
    exiftool(&[
        "-overwrite_original",
        "-GPSLongitudeRef=",
        path.to_str().expect("scratch path is utf-8"),
    ]);

    let data = extract(&path);
    assert!(data.latitude.is_none(), "an incomplete pair is unplaced");
    assert!(data.longitude.is_none(), "longitude has no hemisphere");
}

#[test]
fn incomplete_gps_is_unplaced() {
    let dir = ScratchDir::new("partial-gps");
    let path = write_jpeg(&dir, "partial", 16, 16);
    exiftool(&[
        "-overwrite_original",
        "-GPSLatitude=35",
        "-GPSLatitudeRef=N",
        path.to_str().expect("path"),
    ]);
    let metadata = extract(&path);
    assert_eq!((metadata.latitude, metadata.longitude), (None, None));
}

#[test]
fn gps_reference_must_match_its_axis() {
    use ::exif::{Rational, Tag, Value};
    let coordinate = Value::Rational(vec![
        Rational::from((35, 1)),
        Rational::from((0, 1)),
        Rational::from((0, 1)),
    ]);
    for (tag, wrong_reference) in [
        (Tag::GPSLatitude, "E"),
        (Tag::GPSLatitude, "W"),
        (Tag::GPSLongitude, "N"),
        (Tag::GPSLongitude, "S"),
    ] {
        assert!(
            coordinate_values(
                &coordinate,
                &Value::Ascii(vec![wrong_reference.as_bytes().to_vec()]),
                tag
            )
            .is_none()
        );
    }
}

#[test]
fn metadata_is_optional_when_the_reader_cannot_rewind() {
    struct Unseekable(std::io::Cursor<Vec<u8>>);
    impl std::io::Read for Unseekable {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl std::io::BufRead for Unseekable {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            self.0.fill_buf()
        }
        fn consume(&mut self, n: usize) {
            self.0.consume(n);
        }
    }
    impl Seek for Unseekable {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::other("reader cannot rewind"))
        }
    }
    let exif = extract_reader(Unseekable(std::io::Cursor::new(vec![0; 12])));
    assert!(exif.latitude.is_none());
    assert!(exif.longitude.is_none());
    assert!(exif.orientation.is_none());
    assert!(exif.captured_at.is_none());
}
