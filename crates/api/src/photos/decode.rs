//! Decoding, including the HEIC path.
//!
//! `image` handles JPEG, PNG and WebP. It has **no HEIC decoder at all** — not
//! a disabled feature, the decoder does not exist, largely because HEIC wraps
//! HEVC and carries patent encumbrance the maintainers declined. HEIC therefore
//! goes through `libheif-rs`, and the two paths converge on `DynamicImage` so
//! that nothing downstream has to care which was used (D9).
//!
//! D8: every function here is CPU-bound and must run under `spawn_blocking`.
//!
//! # Orientation: exactly one rotation, applied by the caller
//!
//! [`decode`] returns the pixels **as the file's own decoder produced them**
//! and applies no EXIF orientation of its own. The caller pairs it with
//! [`apply_orientation`] and the value from
//! [`crate::photos::exif::extract`]:
//!
//! ```text
//! let orientation = exif::extract(path).orientation.unwrap_or(1);
//! let image = apply_orientation(decode(path, format)?, orientation);
//! ```
//!
//! Splitting it that way keeps exactly one place that rotates, which matters
//! because the two decode paths do not start from the same position:
//!
//! * JPEG/PNG/WebP — `image` decodes what is literally in the file and never
//!   consults the EXIF orientation, so the tag still has to be applied.
//! * HEIC — libheif carries out the container's own `irot`/`imir`
//!   transformation properties during decode, which is what MIAF requires and
//!   is how every libheif-based tool renders an iPhone photo the right way up.
//!   Verified against libheif 1.23.1: a HEIC whose only orientation signal is
//!   an EXIF tag decodes *unrotated*, so the container property is the sole
//!   authority and the pixels arrive already upright.
//!
//! That asymmetry is reconciled in [`crate::photos::exif::extract`], which
//! reports no orientation for an ISO-BMFF container precisely because the
//! decoder has already consumed it. The caller therefore rotates by 1 — a
//! no-op — and an iPhone portrait is turned once rather than twice.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::OnceLock;

use image::{DynamicImage, ImageBuffer, ImageReader, Rgb, Rgba};
use libheif_rs::{ColorSpace, HeifContext, LibHeif, Plane, RgbChroma, StreamReader};

use crate::photos::{ImageFormat, PhotoError};

/// Decodes to pixels, whatever the source format.
///
/// The returned image has **no metadata attached** — that is the point of
/// converging on a pixel buffer. Orientation is not carried forward as a tag
/// either: it is baked into the pixels by [`apply_orientation`], which the
/// caller applies to this result, so the encoder downstream has nothing to
/// preserve even by accident. See the module docs for why the rotation lives
/// one level up rather than in here.
pub fn decode(path: &Path, format: ImageFormat) -> Result<DynamicImage, PhotoError> {
    match format {
        ImageFormat::Heic => decode_heic(path),
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP => decode_with_image(path, format),
    }
}

/// Bakes an EXIF orientation into the pixels.
///
/// Necessary because the variant is stripped of metadata: a photo shot in
/// portrait carries orientation 6 and looks correct only because viewers honor
/// the tag. Strip the tag without rotating the pixels and every portrait photo
/// in the product appears sideways — a bug that looks like a display problem
/// and is actually a pipeline one.
pub fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    // The four mirrored values (2, 4, 5, 7) are where this goes subtly wrong.
    // A mirrored image that is merely rotated has the right dimensions and
    // roughly the right content, so a test that checks only the size passes
    // while text in the photograph reads backwards. Each of the eight is
    // written out rather than derived, because the derivation is the bug.
    //
    // Read the EXIF names as "where row 0 and column 0 of the stored data
    // belong in the displayed image":
    match orientation {
        // 1, top-left: already upright.
        1 => image,
        // 2, top-right: mirrored across the vertical axis.
        2 => image.fliph(),
        // 3, bottom-right: upside down.
        3 => image.rotate180(),
        // 4, bottom-left: mirrored across the horizontal axis.
        4 => image.flipv(),
        // 5, left-top: mirrored, then rotated 270° clockwise — the transpose.
        // Not the same as 6, and not the same as 8.
        5 => image.rotate90().fliph(),
        // 6, right-top: rotated 90° clockwise. The common portrait case.
        6 => image.rotate90(),
        // 7, right-bottom: mirrored, then rotated 90° clockwise — the
        // anti-transpose.
        7 => image.rotate270().fliph(),
        // 8, left-bottom: rotated 270° clockwise. The other portrait case.
        8 => image.rotate270(),
        // 0 and anything above 8 are not orientations. Buggy writers emit them,
        // and guessing would be worse than leaving the pixels alone: an
        // unrotated photo is visibly wrong and fixable, an arbitrarily rotated
        // one is indistinguishable from a photo that was always like that.
        _ => image,
    }
}

/// JPEG, PNG and WebP, via the pure-Rust decoders.
fn decode_with_image(path: &Path, format: ImageFormat) -> Result<DynamicImage, PhotoError> {
    let file = File::open(path).map_err(PhotoError::Storage)?;

    // A `BufReader` rather than `std::fs::read`: every decoder here consumes
    // its input as a stream, so a 25 MB JPEG never needs a 25 MB `Vec` sitting
    // alongside the pixel buffer it decodes into. Dimensions were already
    // bounded upstream (validate::check_pixel_budget), and `image`'s default
    // allocation limit stays in place as a second floor under that.
    let reader = ImageReader::with_format(BufReader::new(file), image_format(format));

    reader
        .decode()
        .map_err(|err| PhotoError::Undecodable(err.to_string()))
}

/// The `image` crate's format tag for a format we have already sniffed.
///
/// Passed explicitly rather than letting `image` guess again: the format came
/// from a magic-byte sniff (D7), and re-deriving it from the same bytes would
/// only add a second opinion to disagree with the first.
fn image_format(format: ImageFormat) -> image::ImageFormat {
    match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::WebP => image::ImageFormat::WebP,
        // Unreachable by construction — `decode` routes HEIC to libheif before
        // this is called — but returning a lie here would be worse than the
        // arm never being taken, so it is spelled out.
        ImageFormat::Heic => image::ImageFormat::Avif,
    }
}

/// The libheif path (D9).
fn decode_heic(path: &Path) -> Result<DynamicImage, PhotoError> {
    let file = File::open(path).map_err(PhotoError::Storage)?;
    let size = file.metadata().map_err(PhotoError::Storage)?.len();

    // libheif reads through our reader, seeking to the boxes it needs. The file
    // is never slurped into a `Vec` — which matters more here than elsewhere,
    // because a HEIC container can hold several images and libheif only ever
    // pulls the bytes for the one we ask for.
    let reader = StreamReader::new(BufReader::new(file), size);
    let context = HeifContext::read_from_reader(Box::new(reader)).map_err(undecodable)?;

    // The primary item, not item zero: a HEIC from a phone routinely contains a
    // thumbnail, a depth map, and the "live photo" frames alongside the picture
    // the user took, and `pitm` is what says which one is the photograph.
    let handle = context.primary_image_handle().map_err(undecodable)?;

    // Ask libheif for 8-bit interleaved pixels and let it do the YCbCr → RGB
    // conversion and any 10-bit downconversion. Requesting RGBA only when the
    // file has alpha keeps an opaque photo three bytes per pixel instead of
    // four all the way through resize and encode.
    let color_space = if handle.has_alpha_channel() {
        ColorSpace::Rgb(RgbChroma::Rgba)
    } else {
        ColorSpace::Rgb(RgbChroma::Rgb)
    };

    // Default decoding options, which means `ignore_transformations` stays
    // false and libheif applies the container's irot/imir. That is what makes
    // the returned pixels upright; see the module docs.
    let decoded = libheif()
        .decode(&handle, color_space, None)
        .map_err(undecodable)?;

    image_from_interleaved(decoded.planes().interleaved)
}

/// Copies the native plane into an owned image, validating dimensions and
/// dropping row padding before constructing the image buffer.
fn image_from_interleaved(plane: Option<Plane<&[u8]>>) -> Result<DynamicImage, PhotoError> {
    let Plane {
        width,
        height,
        storage_bits_per_pixel,
        stride,
        data,
        ..
    } = plane.ok_or_else(|| {
        PhotoError::Undecodable("HEIC decoded without an interleaved plane".into())
    })?;
    let channels = usize::from(storage_bits_per_pixel / 8);
    if width == 0 || height == 0 || !matches!(channels, 3 | 4) {
        return Err(PhotoError::Undecodable(format!(
            "HEIC decoded to an unusable {width}x{height} plane with {channels} bytes per pixel"
        )));
    }

    // Rows are padded to `stride`, which is not `width * channels`. Copying the
    // whole plane and hoping is how you get a sheared image on any file whose
    // width is not a multiple of the decoder's alignment.
    let row_len = width as usize * channels;
    if stride < row_len {
        return Err(PhotoError::Undecodable(
            "HEIC plane stride is shorter than one row".into(),
        ));
    }

    let mut pixels = Vec::with_capacity(row_len * height as usize);
    for row in data.chunks_exact(stride).take(height as usize) {
        pixels.extend_from_slice(&row[..row_len]);
    }
    if pixels.len() != row_len * height as usize {
        return Err(PhotoError::Undecodable(
            "HEIC plane held fewer rows than its height".into(),
        ));
    }

    let image = if channels == 4 {
        ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, pixels).map(DynamicImage::ImageRgba8)
    } else {
        ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8)
    };

    // Width, height, channel count, and exact buffer length were checked above.
    Ok(image.expect("validated interleaved pixel buffer"))
}

/// The process-wide libheif handle.
///
/// `LibHeif::new()` calls `heif_init` and its `Drop` calls `heif_deinit`, which
/// touch static state inside the C library. Creating one per decode would run
/// that pair on every upload, from whichever blocking thread happened to pick
/// the job up. One value that is created once and never dropped removes the
/// question entirely, and the library's own documentation says a single
/// instance is intended to serve the whole program.
fn libheif() -> &'static LibHeif {
    static LIBHEIF: OnceLock<LibHeif> = OnceLock::new();
    LIBHEIF.get_or_init(LibHeif::new)
}

/// A libheif failure is a property of the file, not of the service: the bytes
/// passed the magic-byte sniff and then would not decode.
fn undecodable(err: libheif_rs::HeifError) -> PhotoError {
    PhotoError::Undecodable(err.to_string())
}

/// Test fixtures shared with sibling photo test modules.
#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::testutil;
