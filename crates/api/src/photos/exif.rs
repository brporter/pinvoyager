//! Extracts optional metadata from original images. Missing or unreadable
//! metadata does not reject an upload. File reads and parsing are synchronous;
//! callers run this work on the blocking pool.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone as _, Utc};

#[derive(Debug, Clone, Default)]
pub struct ExifData {
    pub captured_at: Option<DateTime<Utc>>,
    /// **Exact** coordinates as recorded. Granularity is applied at
    /// serialization, not here — rounding at ingest would discard
    /// precision irreversibly and make a later granularity change impossible.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// EXIF orientation still needed after decoding, when valid (1–8).
    /// Omitted for HEIC/HEIF because libheif applies container transformations
    /// and the caller must not apply the EXIF tag again.
    pub orientation: Option<u16>,
}

/// Reads optional capture time, coordinates, and orientation from an original.
/// File-open failures or missing/unreadable EXIF return defaults;
/// readable fields are retained when other fields are malformed.
pub fn extract(path: &Path) -> ExifData {
    let Ok(file) = File::open(path) else {
        return ExifData::default();
    };

    // Buffered, and handed to the reader as a stream: `read_from_container`
    // seeks to the EXIF segment and parses that alone, so a 25 MB original
    // never becomes a 25 MB `Vec` just to read a timestamp out of its header.
    extract_reader(BufReader::new(file))
}

/// A reader boundary lets tests exercise seek failures without relying on
/// platform-specific special files.
fn extract_reader(mut reader: impl std::io::BufRead + Seek) -> ExifData {
    // Whether the pixels the decoder produces have already been straightened
    // for us. Peeked from the first twelve bytes before anything else reads
    // them, and rewound so the EXIF parser still sees a file from byte zero.
    let decoder_owns_orientation = is_iso_bmff(&mut reader);
    if reader.seek(SeekFrom::Start(0)).is_err() {
        return ExifData::default();
    }

    // Keep successfully parsed tags even when other EXIF fields are malformed.
    let parsed = ::exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut reader)
        .or_else(|err| err.distill_partial_result(|_ignored| ()));

    let exif = match parsed {
        Ok(exif) => exif,
        Err(err) => {
            // Missing or unreadable EXIF is logged at debug and yields empty metadata.
            tracing::debug!(error = %err, "no readable EXIF; treating the photo as unannotated");
            return ExifData::default();
        }
    };

    // A location is useful only as a complete pair; the database enforces the
    // same invariant. Partial camera metadata leaves the photo unplaced.
    let location =
        coordinate(&exif, ::exif::Tag::GPSLatitude, ::exif::Tag::GPSLatitudeRef).zip(coordinate(
            &exif,
            ::exif::Tag::GPSLongitude,
            ::exif::Tag::GPSLongitudeRef,
        ));
    ExifData {
        captured_at: captured_at(&exif),
        latitude: location.map(|(latitude, _)| latitude),
        longitude: location.map(|(_, longitude)| longitude),
        orientation: if decoder_owns_orientation {
            None
        } else {
            orientation(&exif)
        },
    }
}

/// Checks for an ISO-BMFF `ftyp` box at offset four. Returns false if the
/// reader cannot read the twelve-byte header. Used on validated upload formats
/// to decide whether container transformations replace EXIF orientation.
fn is_iso_bmff(reader: &mut impl Read) -> bool {
    let mut header = [0u8; 12];
    // A file too short to hold a box header is not a container; whatever it is,
    // its EXIF orientation is the caller's to apply.
    matches!(reader.read_exact(&mut header), Ok(())) && &header[4..8] == b"ftyp"
}

/// Reads capture time, preferring DateTimeOriginal over DateTime.
/// Uses the matching offset when valid; otherwise treats the wall-clock value
/// as UTC. Returns `None` when no supported date can be parsed.
fn captured_at(exif: &::exif::Exif) -> Option<DateTime<Utc>> {
    let (tag, ifd) = [
        (::exif::Tag::DateTimeOriginal, ::exif::In::PRIMARY),
        (::exif::Tag::DateTimeDigitized, ::exif::In::PRIMARY),
        (::exif::Tag::DateTime, ::exif::In::PRIMARY),
    ]
    .into_iter()
    .find(|(tag, ifd)| exif.get_field(*tag, *ifd).is_some())?;

    let field = exif.get_field(tag, ifd)?;
    let offset_tag = offset_time_tag(tag);
    let offset_value = exif.get_field(offset_tag, ifd).map(|field| &field.value);

    captured_at_values(&field.value, offset_value)
}

fn offset_time_tag(tag: ::exif::Tag) -> ::exif::Tag {
    match tag {
        ::exif::Tag::DateTimeOriginal => ::exif::Tag::OffsetTimeOriginal,
        ::exif::Tag::DateTimeDigitized => ::exif::Tag::OffsetTimeDigitized,
        _ => ::exif::Tag::OffsetTime,
    }
}

fn captured_at_values(
    value: &::exif::Value,
    offset_value: Option<&::exif::Value>,
) -> Option<DateTime<Utc>> {
    let ::exif::Value::Ascii(ref values) = *value else {
        return None;
    };
    let mut datetime = ::exif::DateTime::from_ascii(values.first()?).ok()?;

    // EXIF datetimes are wall-clock with no zone. `OffsetTime*` (Exif 2.31) is
    // the only tag that says which zone that wall clock was in, so when it is
    // present it is the difference between an instant and a guess.
    if let Some(::exif::Value::Ascii(values)) = offset_value
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
        // No usable offset: interpret the recorded wall-clock time as UTC.
        None => Utc.from_local_datetime(&naive).single(),
    }
}

/// Reads a GPS axis as degrees/minutes/seconds and applies its hemisphere sign.
/// Returns `None` for missing or malformed values, unrecognized references,
/// non-finite results, or coordinates outside the axis bounds.
fn coordinate(exif: &::exif::Exif, value_tag: ::exif::Tag, ref_tag: ::exif::Tag) -> Option<f64> {
    let field = exif.get_field(value_tag, ::exif::In::PRIMARY)?;
    let reference = exif.get_field(ref_tag, ::exif::In::PRIMARY)?;
    coordinate_values(&field.value, &reference.value, value_tag)
}

fn coordinate_values(
    value: &::exif::Value,
    reference: &::exif::Value,
    value_tag: ::exif::Tag,
) -> Option<f64> {
    let ::exif::Value::Rational(ref dms) = *value else {
        return None;
    };
    let [degrees, minutes, seconds, ..] = dms.as_slice() else {
        return None;
    };

    let magnitude = degrees.to_f64() + minutes.to_f64() / 60.0 + seconds.to_f64() / 3600.0;
    if !magnitude.is_finite() {
        return None;
    }

    let ::exif::Value::Ascii(ref values) = *reference else {
        return None;
    };
    // Reject an unrecognized hemisphere reference.
    let signed = match (value_tag, values.first()?.as_slice()) {
        (::exif::Tag::GPSLatitude, b"N") | (::exif::Tag::GPSLongitude, b"E") => magnitude,
        (::exif::Tag::GPSLatitude, b"S") | (::exif::Tag::GPSLongitude, b"W") => -magnitude,
        _ => return None,
    };

    // Reject out-of-range coordinates instead of inventing a position by clamping.
    let limit = if value_tag == ::exif::Tag::GPSLatitude {
        90.0
    } else {
        180.0
    };
    (signed.abs() <= limit).then_some(signed)
}

/// Returns a valid orientation from 1–8, or `None` for missing or invalid values.
fn orientation(exif: &::exif::Exif) -> Option<u16> {
    let field = exif.get_field(::exif::Tag::Orientation, ::exif::In::PRIMARY)?;
    let value = u16::try_from(field.value.get_uint(0)?).ok()?;
    (1..=8).contains(&value).then_some(value)
}

#[cfg(test)]
#[path = "exif_tests.rs"]
mod tests;
