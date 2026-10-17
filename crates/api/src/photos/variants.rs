//! Resizes decoded pixels and encodes lossy WebP variants without original
//! EXIF, XMP, or color-profile metadata. Metadata omission is checked in tests.
//! Run resize and encoding work on the blocking pool.

use std::borrow::Cow;

use image::DynamicImage;
use image::imageops::FilterType;

use crate::photos::PhotoError;

/// Lanczos3 resampling filter used when reducing image dimensions.
const FILTER: FilterType = FilterType::Lanczos3;

/// The three sizes served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantKind {
    /// Grid and filmstrip.
    Thumb,
    /// Voyage detail and the photo rail.
    Display,
    /// Lightbox.
    Full,
}

impl VariantKind {
    pub const ALL: [VariantKind; 3] = [VariantKind::Thumb, VariantKind::Display, VariantKind::Full];

    /// Path segment and URL component.
    pub fn as_str(self) -> &'static str {
        match self {
            VariantKind::Thumb => "thumb",
            VariantKind::Display => "display",
            VariantKind::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "thumb" => Some(VariantKind::Thumb),
            "display" => Some(VariantKind::Display),
            "full" => Some(VariantKind::Full),
            _ => None,
        }
    }

    /// Longest edge in pixels. Aspect ratio is preserved and images are never
    /// enlarged — upscaling a small original produces a bigger file that looks
    /// worse.
    pub fn max_edge(self) -> u32 {
        match self {
            VariantKind::Thumb => 400,
            VariantKind::Display => 1200,
            VariantKind::Full => 2560,
        }
    }
}

/// Resizes to the variant's maximum edge and encodes lossy WebP without
/// original metadata. Returns encoding errors as `PhotoError`.
pub fn encode_variant(
    image: &DynamicImage,
    kind: VariantKind,
    quality: f32,
) -> Result<Vec<u8>, PhotoError> {
    let resized =
        tracing::info_span!("photo.resize").in_scope(|| fit_within(image, kind.max_edge()));
    tracing::info_span!("photo.encode", photo.webp_quality = quality)
        .in_scope(|| encode_webp(resized.as_ref(), quality))
}

/// Scales down to fit the maximum edge while preserving aspect ratio.
/// Borrows the original when it already fits, avoiding upscaling and a buffer copy.
fn fit_within(image: &DynamicImage, max_edge: u32) -> Cow<'_, DynamicImage> {
    if image.width() <= max_edge && image.height() <= max_edge {
        return Cow::Borrowed(image);
    }

    // `resize` fits the image *inside* the box rather than filling it, so the
    // aspect ratio is preserved and the long edge lands on `max_edge`. Passing
    // the same value for both dimensions is how you say "longest edge".
    Cow::Owned(image.resize(max_edge, max_edge, FILTER))
}

/// Encodes pixels as lossy WebP without copying original metadata.
/// Clamps quality to 0–100. Invalid dimensions or native encoder failures
/// return `PhotoError::Unencodable`.
fn encode_webp(image: &DynamicImage, quality: f32) -> Result<Vec<u8>, PhotoError> {
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err(PhotoError::Unencodable(format!(
            "cannot encode a {width}x{height} image"
        )));
    }

    // libwebp treats quality outside 0–100 as a usage error and fails the
    // encode. Clamping turns a misconfiguration into a slightly-wrong-looking
    // photo rather than an upload that never becomes viewable.
    let quality = quality.clamp(0.0, 100.0);

    // Keep alpha only if the source has it: an RGBA buffer for an opaque
    // photograph costs a third more memory through the encoder and buys an
    // alpha channel of solid 255s.
    let encoded = if image.color().has_alpha() {
        let buffer = image.to_rgba8();
        webp::Encoder::from_rgba(buffer.as_raw(), width, height).encode_simple(false, quality)
    } else {
        let buffer = image.to_rgb8();
        webp::Encoder::from_rgb(buffer.as_raw(), width, height).encode_simple(false, quality)
    }
    // `encode_simple` rather than `encode`: the latter unwraps internally, and
    // a libwebp failure on a decoded buffer should surface as a failed variant,
    // not as a panic on a blocking thread.
    .map_err(|err| PhotoError::Unencodable(format!("libwebp rejected the image: {err:?}")))?;

    Ok(encoded.to_vec())
}

#[cfg(test)]
#[path = "variants_tests.rs"]
mod tests;
