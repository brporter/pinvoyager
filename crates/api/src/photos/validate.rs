//! Upload validation.
//!
//! CONTRACT — signatures are fixed; implementation owned by the upload agent.
//!
//! Checks are ordered cheapest-first, and the order is the design: each gate
//! exists so the expensive operation behind it never runs on hostile input.
//!
//! 1. Size cap — before the body is read
//! 2. Magic bytes — before any decoder is handed the file
//! 3. Dimensions from the header — before any full decode
//!
//! The filename extension and the client's `Content-Type` are never consulted
//! for anything. They are attacker-controlled strings that happen to look like
//! metadata.

use std::io::BufReader;
use std::path::Path;

use crate::photos::PhotoError;

/// How many leading bytes [`sniff_format`] needs to reach a verdict.
///
/// Twelve would do — the longest signature checked is WebP's `RIFF....WEBP` —
/// but the caller reads from a stream in whatever chunk it gets, so this is
/// stated as a constant rather than left for each call site to rediscover.
/// Passing more than this is harmless; passing less can only produce `None`.
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
    /// The value stored in `photos.original_format`. Must match the migration's
    /// CHECK constraint.
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

/// Identifies the format from the leading bytes of the file.
///
/// Returns `None` for anything not accepted — including formats that *are*
/// images but are declared non-goals (GIF, TIFF, raw). "It is an image" is not
/// the test; "it is an image we handle" is.
///
/// HEIC detection is the awkward one: it is an ISO-BMFF container, so the magic
/// is an `ftyp` box a few bytes in with a brand of `heic`/`heix`/`mif1`/`msf1`,
/// not a fixed prefix.
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

/// Reads dimensions without decoding pixel data.
///
/// This is the decompression-bomb guard, and it must genuinely avoid the full
/// decode — a 50,000 × 50,000 PNG is a few hundred kilobytes on disk and about
/// 10 GB decoded. Checking a byte-size limit does not catch it.
///
/// `format` is the verdict from [`sniff_format`] and is trusted here rather
/// than re-derived: the sniff is the one place the format is decided, and
/// letting a second guess happen inside the decoder would mean two answers that
/// can disagree.
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
    // u64 is the entire point of this line. PNG allows either dimension up to
    // `u32::MAX`, so `width * height` in u32 wraps on hostile input — and a
    // wrapped product is a *small* number, which turns the bomb this function
    // exists to stop into a pass. There is no arithmetic here that can overflow
    // u64: the largest possible product is under 2^64.
    let pixels = u64::from(width) * u64::from(height);
    let budget = u64::from(max_megapixels) * PIXELS_PER_MEGAPIXEL;

    if pixels > budget {
        return Err(PhotoError::TooManyPixels);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed header for each accepted format, long enough for the
    /// sniffer but not a valid file.
    fn jpeg_header() -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        bytes.extend_from_slice(b"JFIF\0\x01\x02\0\0");
        bytes
    }

    fn png_header() -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes
    }

    fn webp_header() -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&64u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBPVP8 ");
        bytes
    }

    /// `ftyp` box: a big-endian length, the box type, then the major brand.
    fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 0x18];
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(brand);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"mif1heic");
        bytes
    }

    #[test]
    fn accepted_formats_are_recognized_by_their_magic() {
        assert_eq!(sniff_format(&jpeg_header()), Some(ImageFormat::Jpeg));
        assert_eq!(sniff_format(&png_header()), Some(ImageFormat::Png));
        assert_eq!(sniff_format(&webp_header()), Some(ImageFormat::WebP));

        for brand in HEIC_BRANDS {
            assert_eq!(
                sniff_format(&ftyp(brand)),
                Some(ImageFormat::Heic),
                "brand {} must sniff as HEIC",
                String::from_utf8_lossy(brand)
            );
        }
    }

    /// The acceptance criterion from the phase plan: a `.jpg` whose bytes are
    /// actually HTML is rejected. The extension never reaches this function,
    /// which is exactly why it cannot help the attacker.
    #[test]
    fn html_pretending_to_be_a_jpeg_is_rejected() {
        let html = b"<!DOCTYPE html>\n<html><body><script>alert(1)</script>";
        assert_eq!(sniff_format(html), None);

        // The same file with a UTF-8 BOM, and with leading whitespace, since
        // both are things a browser will still happily sniff as HTML.
        assert_eq!(sniff_format(b"\xEF\xBB\xBF<html>..........."), None);
        assert_eq!(sniff_format(b"\n\n  <html>.........."), None);
    }

    /// "It is an image" is not the test. GIF, TIFF and raw are declared
    /// non-goals, so they must be refused rather than handed to a decoder.
    #[test]
    fn images_we_do_not_handle_are_rejected() {
        assert_eq!(sniff_format(b"GIF89a\0\0\0\0\0\0\0\0\0\0"), None);
        assert_eq!(sniff_format(b"GIF87a\0\0\0\0\0\0\0\0\0\0"), None);
        // TIFF, both endiannesses — also the container Canon CR2 uses.
        assert_eq!(sniff_format(b"II\x2A\x00\x10\0\0\0CR\x02\0"), None);
        assert_eq!(sniff_format(b"MM\x00\x2A\0\0\0\x10\0\0\0\0"), None);
        // BMP, and a PDF for good measure.
        assert_eq!(sniff_format(b"BM\0\0\0\0\0\0\0\0\0\0"), None);
        assert_eq!(sniff_format(b"%PDF-1.7\0\0\0\0"), None);
    }

    /// AVIF and MP4 share HEIC's container. Matching `ftyp` alone would accept
    /// both, and `original_format` has no spelling for either.
    #[test]
    fn other_iso_bmff_formats_are_rejected() {
        assert_eq!(sniff_format(&ftyp(b"avif")), None);
        assert_eq!(sniff_format(&ftyp(b"avis")), None);
        assert_eq!(sniff_format(&ftyp(b"isom")), None);
        assert_eq!(sniff_format(&ftyp(b"mp42")), None);
        assert_eq!(sniff_format(&ftyp(b"qt  ")), None);
        // Canon CR3 raw — a declared non-goal that is nonetheless ISO-BMFF.
        assert_eq!(sniff_format(&ftyp(b"crx ")), None);
    }

    /// A truncated read must not panic and must not guess. Every slice here is
    /// a prefix of something valid.
    #[test]
    fn short_headers_are_rejected_rather_than_panicking() {
        assert_eq!(sniff_format(&[]), None);
        assert_eq!(sniff_format(&[0xFF]), None);
        assert_eq!(sniff_format(&[0xFF, 0xD8]), None);
        assert_eq!(sniff_format(&PNG_MAGIC[..4]), None);
        assert_eq!(sniff_format(b"RIFF\x40\0\0\0WEB"), None);
        assert_eq!(sniff_format(b"\0\0\0\x18ftyphei"), None);
    }

    /// A JPEG signature is three bytes, so the smallest input that can be
    /// identified is three bytes. Guard against a future rewrite that quietly
    /// requires more.
    #[test]
    fn a_header_of_exactly_the_signature_length_is_enough() {
        assert_eq!(sniff_format(JPEG_MAGIC), Some(ImageFormat::Jpeg));
        assert_eq!(sniff_format(PNG_MAGIC), Some(ImageFormat::Png));
    }

    #[test]
    fn format_spellings_match_the_check_constraint() {
        // These four strings appear verbatim in the photos migration's CHECK.
        assert_eq!(ImageFormat::Jpeg.as_str(), "jpeg");
        assert_eq!(ImageFormat::Png.as_str(), "png");
        assert_eq!(ImageFormat::WebP.as_str(), "webp");
        assert_eq!(ImageFormat::Heic.as_str(), "heic");
    }

    #[test]
    fn an_ordinary_photograph_is_within_budget() {
        // 48 MP, comfortably under the 100 MP default.
        assert!(check_pixel_budget(8000, 6000, 100).is_ok());
        assert!(check_pixel_budget(1, 1, 100).is_ok());
    }

    #[test]
    fn the_budget_boundary_is_inclusive() {
        // Exactly at the limit passes; one pixel over does not.
        assert!(check_pixel_budget(1000, 1000, 1).is_ok());
        assert!(matches!(
            check_pixel_budget(1000, 1001, 1),
            Err(PhotoError::TooManyPixels)
        ));
    }

    /// The decompression bomb from the acceptance criteria: 2.5 gigapixels,
    /// a few hundred kilobytes on disk, about 10 GB decoded.
    #[test]
    fn a_decompression_bomb_is_rejected_on_dimensions() {
        assert!(matches!(
            check_pixel_budget(50_000, 50_000, 100),
            Err(PhotoError::TooManyPixels)
        ));
    }

    /// The reason the arithmetic is in u64.
    ///
    /// `65_536 * 65_536` is exactly `2^32`, which is `0` in wrapping u32 and
    /// `u32::MAX * u32::MAX` wraps to `1`. Either would sail past a budget
    /// check done in u32 — a four-gigapixel image reported as one pixel. PNG
    /// permits both dimensions up to `u32::MAX`, so these are values a hostile
    /// file can actually declare in its IHDR.
    #[test]
    fn a_product_that_would_overflow_u32_is_still_rejected() {
        assert_eq!(65_536u32.wrapping_mul(65_536), 0, "premise of this test");
        assert!(matches!(
            check_pixel_budget(65_536, 65_536, 100),
            Err(PhotoError::TooManyPixels)
        ));

        assert_eq!(u32::MAX.wrapping_mul(u32::MAX), 1, "premise of this test");
        assert!(matches!(
            check_pixel_budget(u32::MAX, u32::MAX, u32::MAX),
            Err(PhotoError::TooManyPixels)
        ));
    }

    /// A zero-megapixel ceiling rejects everything rather than dividing by
    /// zero or wrapping. Not a configuration anyone should set, but "misconfigured"
    /// must not mean "unbounded".
    #[test]
    fn a_zero_budget_admits_nothing() {
        assert!(matches!(
            check_pixel_budget(1, 1, 0),
            Err(PhotoError::TooManyPixels)
        ));
    }

    /// Scratch directory for the probe tests, which need real encoded files.
    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("pinvoyager-validate-tests")
            .join(format!("{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn dimensions_are_read_from_a_real_png() {
        let dir = scratch_dir("png");
        let path = dir.join("probe.png");

        let buffer = image::RgbImage::new(37, 11);
        buffer
            .save_with_format(&path, image::ImageFormat::Png)
            .expect("write png");

        assert_eq!(
            probe_dimensions(&path, ImageFormat::Png).expect("probe png"),
            (37, 11)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dimensions_are_read_from_a_real_jpeg() {
        let dir = scratch_dir("jpeg");
        let path = dir.join("probe.jpg");

        let buffer = image::RgbImage::new(23, 41);
        buffer
            .save_with_format(&path, image::ImageFormat::Jpeg)
            .expect("write jpeg");

        assert_eq!(
            probe_dimensions(&path, ImageFormat::Jpeg).expect("probe jpeg"),
            (23, 41)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dimensions_are_read_from_a_real_webp() {
        let dir = scratch_dir("webp");
        let path = dir.join("probe.webp");

        // `image`'s WebP encoder is lossless-only, which is exactly why
        // variants go through libwebp instead (D9). It is fine for a fixture.
        let buffer = image::RgbaImage::new(64, 17);
        buffer
            .save_with_format(&path, image::ImageFormat::WebP)
            .expect("write webp");

        assert_eq!(
            probe_dimensions(&path, ImageFormat::WebP).expect("probe webp"),
            (64, 17)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The probe must fail rather than succeed on a file whose bytes do not
    /// match the format it was told to expect. This is the second line of
    /// defense behind the sniff: if the two ever disagree, the answer is a
    /// rejection, not a guess.
    #[test]
    fn probing_a_file_that_is_not_the_claimed_format_fails() {
        let dir = scratch_dir("mismatch");
        let path = dir.join("not-really.png");
        std::fs::write(&path, b"<!DOCTYPE html><html></html>").expect("write html");

        assert!(matches!(
            probe_dimensions(&path, ImageFormat::Png),
            Err(PhotoError::Undecodable(_))
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn probing_a_missing_file_is_a_storage_error() {
        let missing = scratch_dir("missing").join("nothing-here.png");

        assert!(matches!(
            probe_dimensions(&missing, ImageFormat::Png),
            Err(PhotoError::Storage(_))
        ));
    }

    /// Writes a small HEIC, or returns `None` when this libheif build has no
    /// HEVC encoder plugin.
    ///
    /// Encoding HEIC needs x265, which is a separate plugin from the decoder
    /// and is genuinely absent in some distributions' libheif packages —
    /// including, deliberately, ones that ship a decode-only build for patent
    /// reasons (D9). Making the fixture optional keeps the suite honest on
    /// those machines instead of failing on something the product never does.
    fn write_heic(path: &Path, width: u32, height: u32) -> Option<()> {
        use libheif_rs::{
            Channel, ColorSpace, CompressionFormat, EncoderQuality, HeifContext, Image, LibHeif,
            RgbChroma,
        };

        let lib = LibHeif::new();
        let mut encoder = lib.encoder_for_format(CompressionFormat::Hevc).ok()?;
        encoder.set_quality(EncoderQuality::Lossy(50)).ok()?;

        let mut image = Image::new(width, height, ColorSpace::Rgb(RgbChroma::Rgb)).ok()?;
        image
            .create_plane(Channel::Interleaved, width, height, 8)
            .ok()?;
        if let Some(plane) = image.planes_mut().interleaved {
            plane.data.fill(0x40);
        }

        let mut context = HeifContext::new().ok()?;
        context.encode_image(&image, &mut encoder, None).ok()?;
        context.write_to_file(path.to_str()?).ok()?;

        Some(())
    }

    /// The HEIC path, end to end: a file libheif wrote, sniffed from its magic
    /// bytes and then probed for dimensions through libheif's container parse.
    /// Both halves matter — an `ftyp` brand this module does not recognize
    /// would make the probe unreachable no matter how correct it is.
    #[test]
    fn dimensions_are_read_from_a_real_heic() {
        let dir = scratch_dir("heic");
        let path = dir.join("probe.heic");

        let Some(()) = write_heic(&path, 40, 24) else {
            eprintln!(
                "skipping: this libheif build has no HEVC encoder, so no HEIC \
                 fixture can be generated. Decoding is unaffected."
            );
            std::fs::remove_dir_all(&dir).ok();
            return;
        };

        let mut header = vec![0u8; HEADER_BYTES];
        let written = std::fs::read(&path).expect("read heic");
        header.copy_from_slice(&written[..HEADER_BYTES]);
        assert_eq!(
            sniff_format(&header),
            Some(ImageFormat::Heic),
            "libheif's own output must sniff as HEIC; got brand {:?}",
            String::from_utf8_lossy(&header[8..12])
        );

        assert_eq!(
            probe_dimensions(&path, ImageFormat::Heic).expect("probe heic"),
            (40, 24)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The HEIC branch must reject rather than panic when handed something
    /// that is not a HEIC. This is the branch that goes through C, so "fails
    /// cleanly" is worth asserting rather than assuming.
    #[test]
    fn probing_a_non_heic_through_the_heic_path_fails_cleanly() {
        let dir = scratch_dir("heic-mismatch");
        let path = dir.join("not-really.heic");
        std::fs::write(&path, b"<!DOCTYPE html><html></html>").expect("write html");

        assert!(matches!(
            probe_dimensions(&path, ImageFormat::Heic),
            Err(PhotoError::Undecodable(_))
        ));

        std::fs::remove_dir_all(&dir).ok();
    }
}
