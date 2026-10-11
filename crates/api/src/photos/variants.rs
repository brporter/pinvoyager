//! Derived variants: resized, re-encoded, stripped.
//!
//! # The strip is the feature
//!
//! D6 requires every served variant to omit original metadata.
//! Published voyages are public, so a served image is a public artifact. If someone
//! chooses ~50 km granularity and the served WebP still carries its original
//! GPS EXIF, the granularity control is decorative and anyone who downloads the
//! file reads the exact coordinates out of it.
//!
//! Encoding from a decoded pixel buffer is what makes the strip structural
//! rather than a step someone can forget: there is no metadata attached to a
//! `DynamicImage` to carry forward. The `exiftool` test asserts it anyway,
//! because "we re-encoded it" and "we stripped it" are different claims and an
//! encoder upgrade can quietly change which one is true.
//!
//! D8: encoding is CPU-bound. Run under `spawn_blocking`.

use std::borrow::Cow;

use image::DynamicImage;
use image::imageops::FilterType;

use crate::photos::PhotoError;

/// Resampling filter for the downscale.
///
/// Lanczos3 rather than a cheaper triangle filter: these are photographs shown
/// at the size we produce, a 400 px thumbnail is an eighth of the original in
/// each axis, and the softness of a box or triangle filter at that reduction is
/// visible. The cost is CPU, which is already off the runtime by D8.
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

/// Resizes and encodes to lossy WebP with no metadata.
///
/// Lossy specifically: the pure-Rust `image-webp` encoder is lossless-only, and
/// lossless WebP on a photograph is typically *larger* than the source JPEG,
/// which would invert the reason for choosing WebP at all. Hence libwebp (D9).
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

/// Scales down to fit inside a `max_edge` square, preserving aspect ratio.
///
/// Never up. Enlarging a small original does not add detail; it produces a
/// bigger file that looks softer than the thing it came from, and it makes a
/// 200 px scan pretend to be a 2560 px photograph in the lightbox. The
/// borrowed-or-owned return is what keeps that case free rather than a full
/// copy of the buffer.
fn fit_within(image: &DynamicImage, max_edge: u32) -> Cow<'_, DynamicImage> {
    if image.width() <= max_edge && image.height() <= max_edge {
        return Cow::Borrowed(image);
    }

    // `resize` fits the image *inside* the box rather than filling it, so the
    // aspect ratio is preserved and the long edge lands on `max_edge`. Passing
    // the same value for both dimensions is how you say "longest edge".
    Cow::Owned(image.resize(max_edge, max_edge, FILTER))
}

/// Encodes a pixel buffer as lossy WebP through libwebp.
///
/// **This is the strip.** The input is a `DynamicImage` — a width, a height and
/// pixels, with nowhere for an EXIF block, an XMP packet or a color profile to
/// hide — and libwebp's encoder writes only the bitstream chunks it generates
/// itself. There is no metadata to carry forward because there is no metadata
/// in the input. That structural property is what the `exiftool` test verifies
/// rather than trusts.
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
