//! Upload validation.
//!
//! Validate inexpensive properties before decoding so oversized or unsupported
//! input does not trigger expensive image work.
//!
//! 1. Request size limit, checked before reading when Content-Length is present
//!    and enforced while streaming
//! 2. Magic bytes, before any decoder reads the file
//! 3. Header dimensions, before full pixel decoding

use std::io::BufReader;
use std::path::Path;

use crate::photos::PhotoError;

/// Leading-byte window used for format detection. Individual signatures may
/// be recognized with fewer bytes; unrecognized or incomplete signatures return `None`.
pub const HEADER_BYTES: usize = 16;

/// The formats accepted on upload.
///
/// HEIC is here because iPhones shoot it by default; declining it would reject
/// a large share of the photographs this product exists to display (D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Heic,
}

impl ImageFormat {
    /// Lowercase original-format name stored with photo metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Png => "png",
            ImageFormat::WebP => "webp",
            ImageFormat::Heic => "heic",
        }
    }
}

/// PNG's eight-byte signature. The trailing CRLF/EOF bytes are there to catch a
/// file mangled by a text-mode transfer, which is why it is longer than most.
const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Start of Image, immediately followed by the first marker's `FF`. Checking
/// the third byte as well rejects a bare `FF D8` that is not a JPEG stream.
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];

/// ISO-BMFF brands we treat as HEIC.
///
/// `heic`/`heix` are the still-image brands an iPhone writes; `mif1`/`msf1` are
/// the generic HEIF image and image-sequence brands that some encoders use as
/// the major brand instead. Everything else — `avif`, `mp41`, `qt  `, `crx `
/// (Canon raw) — is a different format sharing the same container, and a
/// different format is a rejection, not a decode attempt.
const HEIC_BRANDS: [&[u8; 4]; 4] = [b"heic", b"heix", b"mif1", b"msf1"];

/// Detects JPEG, PNG, WebP, or supported HEIC brands from leading bytes.
/// Returns `None` for unsupported or incomplete signatures; filenames and
/// client MIME types are ignored.
pub fn sniff_format(header: &[u8]) -> Option<ImageFormat> {
    if header.starts_with(JPEG_MAGIC) {
        return Some(ImageFormat::Jpeg);
    }

    if header.starts_with(PNG_MAGIC) {
        return Some(ImageFormat::Png);
    }

    // RIFF container: `RIFF` + a four-byte little-endian length + the form type.
    // The length is not checked — it is self-declared and a lie in it is the
    // decoder's problem, not the sniffer's.
    if header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        return Some(ImageFormat::WebP);
    }

    // ISO-BMFF: a size-prefixed `ftyp` box at offset 0, so the brand sits at 8.
    // There is no fixed prefix to match — the first four bytes are a big-endian
    // box length that varies by how many compatible brands follow.
    if header.len() >= 12 && &header[4..8] == b"ftyp" {
        let brand = &header[8..12];
        if HEIC_BRANDS.iter().any(|candidate| *candidate == brand) {
            return Some(ImageFormat::Heic);
        }
    }

    None
}

/// Reads dimensions using the already-detected format without decoding pixels.
/// Returns file-read or malformed-image errors. Callers check the pixel budget
/// before full decoding.
pub fn probe_dimensions(
    path: &std::path::Path,
    format: ImageFormat,
) -> Result<(u32, u32), PhotoError> {
    match format {
        // `image` has no HEIC decoder at all (D9), so this one goes to libheif.
        ImageFormat::Heic => probe_heic(path),
        ImageFormat::Jpeg => probe_with_image(path, image::ImageFormat::Jpeg),
        ImageFormat::Png => probe_with_image(path, image::ImageFormat::Png),
        ImageFormat::WebP => probe_with_image(path, image::ImageFormat::WebP),
    }
}

/// Header-only dimension read via `image`.
///
/// `into_dimensions` constructs the decoder and asks it for its dimensions
/// without ever requesting pixels, so a decompression bomb costs a header parse
/// rather than the allocation it is trying to provoke.
///
/// Note that `ImageReader::open` is deliberately *not* used: it guesses the
/// format from the file extension, and the whole point of this module is that
/// no filename is trusted. The format is set explicitly from the magic-byte
/// verdict instead.
fn probe_with_image(path: &Path, format: image::ImageFormat) -> Result<(u32, u32), PhotoError> {
    let file = std::fs::File::open(path).map_err(PhotoError::Storage)?;
    let mut reader = image::ImageReader::new(BufReader::new(file));
    reader.set_format(format);

    reader
        .into_dimensions()
        .map_err(|err| PhotoError::Undecodable(err.to_string()))
}

/// Header-only dimension read via libheif.
///
/// `HeifContext` parses the container's box structure; the width and height
/// come from the primary item's `ispe` (image spatial extents) box. No decoder
/// plugin is invoked and no pixel buffer is allocated — that only happens on an
/// explicit `decode`, which lives in [`crate::photos::decode`].
fn probe_heic(path: &Path) -> Result<(u32, u32), PhotoError> {
    // libheif's API takes a C string, so a path that is not valid UTF-8 cannot
    // be handed to it. Unreachable in practice — uploads land on paths this
    // service constructs — but silently mis-decoding is not an option, so it is
    // an error rather than a lossy conversion.
    let path_str = path
        .to_str()
        .ok_or_else(|| PhotoError::Undecodable("HEIC path is not valid UTF-8".to_owned()))?;

    let context = libheif_rs::HeifContext::read_from_file(path_str)
        .map_err(|err| PhotoError::Undecodable(format!("HEIC container: {err}")))?;

    // The primary item specifically. A HEIC from a phone routinely holds a
    // thumbnail, a depth map and an auxiliary alpha image alongside the
    // photograph, and any of those would give the wrong dimensions.
    let handle = context
        .primary_image_handle()
        .map_err(|err| PhotoError::Undecodable(format!("HEIC primary image: {err}")))?;

    Ok((handle.width(), handle.height()))
}

/// One megapixel, decimal — the unit `max_megapixels` is expressed in.
const PIXELS_PER_MEGAPIXEL: u64 = 1_000_000;

/// Rejects an image whose pixel count exceeds the configured ceiling.
pub fn check_pixel_budget(width: u32, height: u32, max_megapixels: u32) -> Result<(), PhotoError> {
    // Multiply in u64 so even the largest u32 dimensions cannot overflow.
    let pixels = u64::from(width) * u64::from(height);
    let budget = u64::from(max_megapixels) * PIXELS_PER_MEGAPIXEL;

    if pixels > budget {
        return Err(PhotoError::TooManyPixels);
    }

    Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
