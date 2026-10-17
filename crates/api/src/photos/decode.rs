//! Decodes JPEG, PNG, and WebP with `image`, and HEIC with libheif.
//! Both paths return `DynamicImage` pixel buffers without original metadata.
//! Run decoding on the blocking pool.
//!
//! Callers apply EXIF orientation separately with [`apply_orientation`]. For
//! HEIC, libheif applies container transformations during decoding and EXIF
//! extraction suppresses the orientation tag to avoid applying it twice.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::OnceLock;

use image::{DynamicImage, ImageBuffer, ImageReader, Rgb, Rgba};
use libheif_rs::{ColorSpace, HeifContext, LibHeif, Plane, RgbChroma, StreamReader};

use crate::photos::{ImageFormat, PhotoError};

/// Decodes an image into pixels without carrying original metadata forward.
/// The caller applies EXIF orientation separately. File-read and decoder
/// failures return `PhotoError`.
pub fn decode(path: &Path, format: ImageFormat) -> Result<DynamicImage, PhotoError> {
    match format {
        ImageFormat::Heic => decode_heic(path),
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP => decode_with_image(path, format),
    }
}

/// Applies an EXIF orientation to the pixel buffer. Handles rotations and
/// mirroring for values 1–8; unrecognized values leave the image unchanged.
pub fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    // Handle mirrored orientations separately from rotations.
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
        // Leave pixels unchanged for invalid orientation values.
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
        // HEIC is routed to libheif by `decode` before this helper is called.
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

    // Copy each row's pixels without the native plane's stride padding.
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

/// Initializes libheif once for all decoding calls. The shared handle remains
/// alive for the process lifetime, avoiding repeated native init/deinit calls.
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
