//! EXIF extraction.
//!
//! CONTRACT — signatures are fixed; implementation owned by the imaging agent.
//!
//! Read from the **original**, never from a variant — variants are stripped by
//! design, so reading one would always yield nothing.
//!
//! Absent data is the normal case, not an error. Screenshots, scans, and any
//! camera with location services off carry no GPS; plenty of images carry no
//! timestamp either. Every field here is optional and the pipeline must not
//! treat a missing one as a failure.
//!
//! D8: this reads and parses a file, so it is blocking work. Callers run it
//! under [`crate::blocking::run_blocking`]; the leaf function itself is
//! deliberately synchronous so it composes with decode and encode inside a
//! single `spawn_blocking` closure rather than hopping the pool three times.

use std::fs::File;
use std::io::{BufReader, Read as _, Seek as _, SeekFrom};
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone as _, Utc};

#[derive(Debug, Clone, Default)]
pub struct ExifData {
    pub captured_at: Option<DateTime<Utc>>,
    /// **Exact** coordinates as recorded. Granularity is applied at
    /// serialization (phase 5), not here — rounding at ingest would discard
    /// precision irreversibly and make a later granularity change impossible.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// EXIF orientation, 1–8. Applied to the pixels during decode and then
    /// discarded.
    ///
    /// Read it as **the rotation the decoded pixels still need**, not as a
    /// verbatim copy of the tag. The two differ for exactly one family of
    /// files: in an ISO-BMFF container (HEIC/HEIF) the authoritative
    /// orientation is the `irot`/`imir` transformation property, which libheif
    /// has already carried out by the time
    /// [`crate::photos::decode::decode`] returns. Reporting the EXIF tag as
    /// well would have the caller rotate a second time and land an iPhone
    /// portrait face down, so it is deliberately not reported. See
    /// [`crate::photos::decode`] for the whole argument.
    pub orientation: Option<u16>,
}

/// Extracts what is present, tolerating everything that is not.
///
/// A malformed or absent EXIF block yields `ExifData::default()` rather than an
/// error: a photo with unreadable metadata is still a photo, and refusing it
/// would reject files every other viewer opens happily.
///
/// GPS in EXIF is degrees/minutes/seconds plus a N/S/E/W reference, so the
/// sign has to be applied from the reference — getting that wrong puts European
/// photographs in the Atlantic and is not obvious from a single test case.
pub fn extract(path: &Path) -> ExifData {
    let Ok(file) = File::open(path) else {
        return ExifData::default();
    };

    // Buffered, and handed to the reader as a stream: `read_from_container`
    // seeks to the EXIF segment and parses that alone, so a 25 MB original
    // never becomes a 25 MB `Vec` just to read a timestamp out of its header.
    let mut reader = BufReader::new(file);

    // Whether the pixels the decoder produces have already been straightened
    // for us. Peeked from the first twelve bytes before anything else reads
    // them, and rewound so the EXIF parser still sees a file from byte zero.
    let decoder_owns_orientation = is_iso_bmff(&mut reader);
    if reader.seek(SeekFrom::Start(0)).is_err() {
        return ExifData::default();
    }

    // `continue_on_error` keeps the fields that did parse when a later one is
    // malformed, and `distill_partial_result` unwraps that partial success into
    // a plain `Exif`. A single bad tag from an unusual camera must not cost us
    // the GPS coordinates sitting next to it — partial data is exactly what
    // this function promises to salvage.
    let parsed = ::exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut reader)
        .or_else(|err| err.distill_partial_result(|_ignored| ()));

    let exif = match parsed {
        Ok(exif) => exif,
        Err(err) => {
            // Debug, not warn: a file with no EXIF at all lands here and that
            // is the ordinary case for screenshots and scans. Worth seeing when
            // chasing a specific upload, never worth alerting on.
            tracing::debug!(error = %err, "no readable EXIF; treating the photo as unannotated");
            return ExifData::default();
        }
    };

    ExifData {
        captured_at: captured_at(&exif),
        latitude: coordinate(&exif, ::exif::Tag::GPSLatitude, ::exif::Tag::GPSLatitudeRef),
        longitude: coordinate(
            &exif,
            ::exif::Tag::GPSLongitude,
            ::exif::Tag::GPSLongitudeRef,
        ),
        orientation: if decoder_owns_orientation {
            None
        } else {
            orientation(&exif)
        },
    }
}

/// Whether the file is an ISO base media container — HEIC and HEIF.
///
/// The marker is a `ftyp` box at offset 4 rather than a fixed prefix, because
/// the first four bytes are the box's length. Uploads have already been
/// narrowed to JPEG, PNG, WebP and HEIC by the magic-byte sniff (D7), so an
/// `ftyp` here is a HEIF and nothing else.
///
/// This exists only to answer "has libheif already applied the rotation?" —
/// see the note on [`ExifData::orientation`].
fn is_iso_bmff(reader: &mut BufReader<File>) -> bool {
    let mut header = [0u8; 12];
    // A file too short to hold a box header is not a container; whatever it is,
    // its EXIF orientation is the caller's to apply.
    matches!(reader.read_exact(&mut header), Ok(())) && &header[4..8] == b"ftyp"
}

/// When the shutter fired, normalized to UTC.
///
/// `DateTimeOriginal` is preferred over `DateTime`: the former is when the
/// picture was taken, the latter is when the file was last written, and an edit
/// in any photo tool moves the second without touching the first.
fn captured_at(exif: &::exif::Exif) -> Option<DateTime<Utc>> {
    let (tag, ifd) = [
        (::exif::Tag::DateTimeOriginal, ::exif::In::PRIMARY),
        (::exif::Tag::DateTimeDigitized, ::exif::In::PRIMARY),
        (::exif::Tag::DateTime, ::exif::In::PRIMARY),
    ]
    .into_iter()
    .find(|(tag, ifd)| exif.get_field(*tag, *ifd).is_some())?;

    let field = exif.get_field(tag, ifd)?;
    let ::exif::Value::Ascii(ref values) = field.value else {
        return None;
    };
    let mut datetime = ::exif::DateTime::from_ascii(values.first()?).ok()?;

    // EXIF datetimes are wall-clock with no zone. `OffsetTime*` (Exif 2.31) is
    // the only tag that says which zone that wall clock was in, so when it is
    // present it is the difference between an instant and a guess.
    let offset_tag = match tag {
        ::exif::Tag::DateTimeOriginal => ::exif::Tag::OffsetTimeOriginal,
        ::exif::Tag::DateTimeDigitized => ::exif::Tag::OffsetTimeDigitized,
        _ => ::exif::Tag::OffsetTime,
    };
    if let Some(field) = exif.get_field(offset_tag, ifd)
        && let ::exif::Value::Ascii(ref values) = field.value
        && let Some(raw) = values.first()
    {
        // A malformed offset leaves `datetime.offset` as `None`, which falls
        // through to the UTC assumption below rather than failing the parse.
        let _ = datetime.parse_offset(raw);
    }

    let date = NaiveDate::from_ymd_opt(
        i32::from(datetime.year),
        u32::from(datetime.month),
        u32::from(datetime.day),
    )?;
    let naive = date.and_hms_nano_opt(
        u32::from(datetime.hour),
        u32::from(datetime.minute),
        u32::from(datetime.second),
        datetime.nanosecond.unwrap_or(0),
    )?;

    match datetime.offset {
        Some(minutes) => FixedOffset::east_opt(i32::from(minutes) * 60)?
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc)),
        // No zone recorded, so there is no honest conversion to make. Treating
        // the wall clock as UTC keeps the stored value equal to what every
        // other tool shows for the photo; inventing a zone from the GPS
        // position would be a guess that silently moves timestamps by hours.
        None => Utc.from_local_datetime(&naive).single(),
    }
}

/// One GPS coordinate, signed by its reference.
///
/// **This is where the bugs live.** EXIF stores an unsigned magnitude as three
/// rationals — degrees, minutes, seconds — and puts the hemisphere in a
/// *separate* tag. Ignore the reference and every southern and western
/// coordinate comes out mirrored into the wrong hemisphere: a Sydney photo
/// lands in Siberia and a London one in the Atlantic. Neither is visible from a
/// test case in the northern-eastern quadrant, because there both signs are
/// positive and the bug is invisible.
///
/// The value is returned **exact**. Granularity is a serialization concern
/// (phase 5); rounding here would throw away precision that cannot be
/// recovered, and would break the future "adjust location" feature.
fn coordinate(exif: &::exif::Exif, value_tag: ::exif::Tag, ref_tag: ::exif::Tag) -> Option<f64> {
    let field = exif.get_field(value_tag, ::exif::In::PRIMARY)?;
    let ::exif::Value::Rational(ref dms) = field.value else {
        return None;
    };
    let [degrees, minutes, seconds] = dms.get(..3)? else {
        return None;
    };

    let magnitude = degrees.to_f64() + minutes.to_f64() / 60.0 + seconds.to_f64() / 3600.0;
    if !magnitude.is_finite() {
        return None;
    }

    let reference = exif.get_field(ref_tag, ::exif::In::PRIMARY)?;
    let ::exif::Value::Ascii(ref values) = reference.value else {
        return None;
    };
    // The reference is a one-character ASCII string. Anything else is a file we
    // cannot interpret, and a coordinate we cannot sign is worse than no
    // coordinate at all — it would place the photo somewhere confidently wrong.
    let signed = match values.first()?.first()? {
        b'N' | b'E' => magnitude,
        b'S' | b'W' => -magnitude,
        _ => return None,
    };

    // A bounds check rather than a clamp: an out-of-range value means the tag
    // was misparsed or the file is lying, and pinning it to the pole would
    // invent a location. Better to report the photo as unplaced.
    let limit = if value_tag == ::exif::Tag::GPSLatitude {
        90.0
    } else {
        180.0
    };
    (signed.abs() <= limit).then_some(signed)
}

/// The orientation tag, validated to the 1–8 the spec defines.
///
/// Values outside that range exist in the wild (0 is common from buggy
/// writers). They are dropped rather than defaulted to 1 so that the decoder
/// can tell "the file says upright" from "the file says nothing".
fn orientation(exif: &::exif::Exif) -> Option<u16> {
    let field = exif.get_field(::exif::Tag::Orientation, ::exif::In::PRIMARY)?;
    let value = u16::try_from(field.value.get_uint(0)?).ok()?;
    (1..=8).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::photos::decode::testutil::{ScratchDir, exiftool, write_jpeg};

    /// Tolerance for a coordinate that has been through exiftool's
    /// decimal → DMS-rational → decimal round trip. Roughly a centimeter;
    /// tight enough that any *rounding* of the value would blow straight
    /// through it.
    const EPSILON: f64 = 1e-9;

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

    /// D6 and the phase plan both turn on coordinates being stored exactly:
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
        assert!(data.latitude.is_some(), "latitude still has its reference");
        assert!(data.longitude.is_none(), "longitude has no hemisphere");
    }
}
