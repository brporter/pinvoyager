//! Decoding, including the HEIC path.
//!
//! CONTRACT — signatures are fixed; implementation owned by the imaging agent.
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
use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma, StreamReader};

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

    let planes = decoded.planes();
    let plane = planes.interleaved.ok_or_else(|| {
        PhotoError::Undecodable("HEIC decoded without an interleaved plane".into())
    })?;

    let width = plane.width;
    let height = plane.height;
    let channels = usize::from(plane.storage_bits_per_pixel / 8);
    if width == 0 || height == 0 || !matches!(channels, 3 | 4) {
        return Err(PhotoError::Undecodable(format!(
            "HEIC decoded to an unusable {width}x{height} plane with {channels} bytes per pixel"
        )));
    }

    // Rows are padded to `stride`, which is not `width * channels`. Copying the
    // whole plane and hoping is how you get a sheared image on any file whose
    // width is not a multiple of the decoder's alignment.
    let row_len = width as usize * channels;
    if plane.stride < row_len {
        return Err(PhotoError::Undecodable(
            "HEIC plane stride is shorter than one row".into(),
        ));
    }

    let mut pixels = Vec::with_capacity(row_len * height as usize);
    for row in plane.data.chunks_exact(plane.stride).take(height as usize) {
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

    image.ok_or_else(|| PhotoError::Undecodable("HEIC pixel buffer was the wrong size".into()))
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

/// Fixtures shared by the tests in this module and its siblings.
///
/// Lives here, rather than in a `tests/support` file, because `photos/mod.rs`
/// is a fixed contract that this agent does not own — a `#[cfg(test)]` module
/// nested inside an already-declared file adds the helpers without touching it.
#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    use image::{DynamicImage, Rgb, RgbImage};

    /// A unique temporary directory that removes itself.
    ///
    /// Hand-rolled because `tempfile` is not a dependency of this crate and
    /// `Cargo.toml` is owned elsewhere.
    pub(crate) struct ScratchDir(PathBuf);

    impl ScratchDir {
        pub(crate) fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pinvoyager-{label}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create scratch dir");
            Self(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An image with no two quadrants alike.
    ///
    /// Asymmetry in *both* axes is the whole point: a square of one color, or
    /// a pattern symmetric about either axis, cannot tell a rotation from a
    /// mirror, which is exactly the confusion the orientation tests exist to
    /// catch.
    pub(crate) fn asymmetric_image(width: u32, height: u32) -> DynamicImage {
        let mut buffer = RgbImage::new(width, height);
        for (x, y, pixel) in buffer.enumerate_pixels_mut() {
            let left = x < width / 2;
            let top = y < height / 2;
            *pixel = match (left, top) {
                (true, true) => Rgb([255, 0, 0]),
                (false, true) => Rgb([0, 255, 0]),
                (true, false) => Rgb([0, 0, 255]),
                (false, false) => Rgb([255, 255, 0]),
            };
        }
        DynamicImage::ImageRgb8(buffer)
    }

    /// The pipeline's decode step in full: read the orientation, decode, and
    /// bake the one into the other.
    ///
    /// Mirrors what `worker::derive` does, so a test that asserts on "the
    /// pixels the encoder sees" is asserting on the real composition rather
    /// than on `decode` in isolation.
    pub(crate) fn decode_upright(path: &Path, format: crate::photos::ImageFormat) -> DynamicImage {
        let orientation = crate::photos::exif::extract(path).orientation.unwrap_or(1);
        super::apply_orientation(super::decode(path, format).expect("decode"), orientation)
    }

    /// Writes an asymmetric JPEG into the scratch directory.
    pub(crate) fn write_jpeg(dir: &ScratchDir, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.path().join(format!("{name}.jpg"));
        asymmetric_image(width, height)
            .save_with_format(&path, image::ImageFormat::Jpeg)
            .expect("write jpeg fixture");
        path
    }

    /// Writes an asymmetric PNG into the scratch directory.
    pub(crate) fn write_png(dir: &ScratchDir, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.path().join(format!("{name}.png"));
        asymmetric_image(width, height)
            .save_with_format(&path, image::ImageFormat::Png)
            .expect("write png fixture");
        path
    }

    /// Runs `exiftool` and returns its stdout.
    ///
    /// Failing rather than skipping is deliberate. The D6 guarantee is asserted
    /// with this tool; a test that quietly passes because the tool is missing
    /// would report "metadata is stripped" without having looked, which is the
    /// precise failure the acceptance criterion exists to prevent.
    pub(crate) fn exiftool(args: &[&str]) -> String {
        let output = Command::new("exiftool").args(args).output().expect(
            "exiftool must be installed to run the photo pipeline tests \
             (brew install exiftool / apt install libimage-exiftool-perl)",
        );
        assert!(
            output.status.success(),
            "exiftool {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use image::{GenericImageView as _, Rgba};

    use testutil::{ScratchDir, asymmetric_image, exiftool, write_jpeg, write_png};

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const GREEN: Rgba<u8> = Rgba([0, 255, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);
    const YELLOW: Rgba<u8> = Rgba([255, 255, 0, 255]);

    /// A 2x2 image whose four pixels are all different, so every one of the
    /// eight transforms produces a distinct arrangement.
    fn quad() -> DynamicImage {
        asymmetric_image(2, 2)
    }

    fn corners(image: &DynamicImage) -> [Rgba<u8>; 4] {
        let (w, h) = image.dimensions();
        [
            image.get_pixel(0, 0),
            image.get_pixel(w - 1, 0),
            image.get_pixel(0, h - 1),
            image.get_pixel(w - 1, h - 1),
        ]
    }

    /// All eight transforms, checked by *where individual pixels land* rather
    /// than by the resulting dimensions.
    ///
    /// Dimensions alone cannot distinguish a rotation from a mirror — 2, 4, 5
    /// and 7 all produce the same shape as some rotation — so an assertion on
    /// size passes for an implementation that renders every mirrored photo
    /// backwards.
    #[test]
    fn every_orientation_moves_pixels_where_the_spec_says() {
        // Source layout, top-left origin:
        //   R G
        //   B Y
        let source = quad();
        assert_eq!(corners(&source), [RED, GREEN, BLUE, YELLOW]);

        // Expected corners, read as [top-left, top-right, bottom-left, bottom-right].
        let cases = [
            // 1 — unchanged.
            (1u16, [RED, GREEN, BLUE, YELLOW]),
            // 2 — mirrored horizontally: the columns swap, the rows do not.
            (2, [GREEN, RED, YELLOW, BLUE]),
            // 3 — 180°: both swap.
            (3, [YELLOW, BLUE, GREEN, RED]),
            // 4 — mirrored vertically: the rows swap, the columns do not.
            (4, [BLUE, YELLOW, RED, GREEN]),
            // 5 — transpose: reflection about the main diagonal. R and Y are on
            // that diagonal and stay put; G and B trade places.
            (5, [RED, BLUE, GREEN, YELLOW]),
            // 6 — 90° clockwise: the left column becomes the top row, bottom to
            // top.
            (6, [BLUE, RED, YELLOW, GREEN]),
            // 7 — anti-transpose: reflection about the other diagonal. G and B
            // stay put; R and Y trade places.
            (7, [YELLOW, GREEN, BLUE, RED]),
            // 8 — 270° clockwise.
            (8, [GREEN, YELLOW, RED, BLUE]),
        ];

        for (orientation, expected) in cases {
            let rotated = apply_orientation(source.clone(), orientation);
            assert_eq!(
                corners(&rotated),
                expected,
                "orientation {orientation} put the pixels in the wrong places"
            );
        }
    }

    /// The mirrored values are the ones that look nearly right when wrong, so
    /// they get their own assertion: each must differ from *every* pure
    /// rotation of the same image.
    #[test]
    fn mirrored_orientations_are_not_merely_rotations() {
        let source = quad();
        let rotations: Vec<[Rgba<u8>; 4]> = [1, 3, 6, 8]
            .into_iter()
            .map(|o| corners(&apply_orientation(source.clone(), o)))
            .collect();

        for mirrored in [2u16, 4, 5, 7] {
            let got = corners(&apply_orientation(source.clone(), mirrored));
            assert!(
                !rotations.contains(&got),
                "orientation {mirrored} produced a pure rotation; the mirror was dropped"
            );
        }
    }

    /// A rotation by 90° or 270° has to swap the axes, and a mirror must not.
    #[test]
    fn quarter_turns_swap_the_axes_and_mirrors_do_not() {
        let source = asymmetric_image(8, 4);

        for orientation in [5u16, 6, 7, 8] {
            let rotated = apply_orientation(source.clone(), orientation);
            assert_eq!(
                rotated.dimensions(),
                (4, 8),
                "orientation {orientation} should transpose the dimensions"
            );
        }

        for orientation in [1u16, 2, 3, 4] {
            let kept = apply_orientation(source.clone(), orientation);
            assert_eq!(
                kept.dimensions(),
                (8, 4),
                "orientation {orientation} should keep the dimensions"
            );
        }
    }

    #[test]
    fn an_invalid_orientation_leaves_the_pixels_alone() {
        let source = asymmetric_image(4, 2);

        for orientation in [0u16, 9, 255, u16::MAX] {
            let result = apply_orientation(source.clone(), orientation);
            assert_eq!(result.dimensions(), source.dimensions());
            assert_eq!(corners(&result), corners(&source));
        }
    }

    #[test]
    fn decodes_png_and_jpeg_and_webp() {
        let dir = ScratchDir::new("decode-formats");
        let source = asymmetric_image(64, 32);

        let png = write_png(&dir, "sample", 64, 32);
        let decoded = decode(&png, ImageFormat::Png).expect("decode png");
        assert_eq!(decoded.dimensions(), (64, 32));
        // PNG is lossless, so this is an exact comparison.
        assert_eq!(corners(&decoded), corners(&source));

        let jpeg = write_jpeg(&dir, "sample", 64, 32);
        let decoded = decode(&jpeg, ImageFormat::Jpeg).expect("decode jpeg");
        assert_eq!(decoded.dimensions(), (64, 32));

        let webp_path = dir.path().join("sample.webp");
        let bytes = crate::photos::variants::encode_variant(
            &source,
            crate::photos::VariantKind::Thumb,
            90.0,
        )
        .expect("encode webp fixture");
        std::fs::write(&webp_path, bytes).expect("write webp fixture");
        let decoded = decode(&webp_path, ImageFormat::WebP).expect("decode webp");
        assert_eq!(decoded.dimensions(), (64, 32));
    }

    /// The portrait case, end to end: a landscape JPEG tagged orientation 6 is
    /// a portrait photograph, and the decode step must hand the encoder
    /// portrait pixels.
    #[test]
    fn the_decode_step_bakes_the_orientation_into_the_pixels() {
        let dir = ScratchDir::new("decode-orientation");
        let path = write_jpeg(&dir, "portrait", 64, 32);
        exiftool(&[
            "-overwrite_original",
            "-Orientation#=6",
            path.to_str().expect("scratch path is utf-8"),
        ]);

        // `decode` alone reports what is in the file — `image` never consults
        // the tag — and the rotation is the caller's step.
        assert_eq!(
            decode(&path, ImageFormat::Jpeg)
                .expect("decode")
                .dimensions(),
            (64, 32)
        );

        let decoded = testutil::decode_upright(&path, ImageFormat::Jpeg);
        assert_eq!(
            decoded.dimensions(),
            (32, 64),
            "orientation 6 must transpose the image"
        );

        // And the content moved with it: the source's bottom-left quadrant is
        // blue, and a 90° clockwise turn puts blue in the top-left.
        let top_left = decoded.get_pixel(4, 4);
        assert!(
            top_left.0[2] > top_left.0[0] && top_left.0[2] > top_left.0[1],
            "expected the blue quadrant at the top left, got {top_left:?}"
        );
    }

    #[test]
    fn an_undecodable_file_is_an_error_not_a_panic() {
        let dir = ScratchDir::new("decode-garbage");
        let path = dir.path().join("not-an-image.jpg");
        std::fs::write(&path, b"<html>this is not a photograph</html>").expect("write garbage");

        assert!(matches!(
            decode(&path, ImageFormat::Jpeg),
            Err(PhotoError::Undecodable(_))
        ));

        let missing = dir.path().join("absent.png");
        assert!(matches!(
            decode(&missing, ImageFormat::Png),
            Err(PhotoError::Storage(_))
        ));
    }

    /// Builds a real HEIC through libheif's own encoder.
    ///
    /// The *encoder* is a test fixture, not a product dependency — we only ever
    /// decode HEIC — and it needs an HEVC plugin that a minimal libheif build
    /// may not ship. `None` means the fixture could not be built, which is a
    /// reason to stop rather than to fail a build.
    fn heic_fixture(width: u32, height: u32) -> Option<Vec<u8>> {
        use libheif_rs::{Channel, CompressionFormat, EncoderQuality, Image as HeifImage};

        let mut encoder = libheif().encoder_for_format(CompressionFormat::Hevc).ok()?;
        encoder
            .set_quality(EncoderQuality::LossLess)
            .expect("set lossless");

        let source = asymmetric_image(width, height).to_rgb8();

        let mut heif_image = HeifImage::new(width, height, ColorSpace::Rgb(RgbChroma::Rgb))
            .expect("create heif image");
        heif_image
            .create_plane(Channel::Interleaved, width, height, 8)
            .expect("create plane");
        {
            let mut planes = heif_image.planes_mut();
            let plane = planes.interleaved.as_mut().expect("interleaved plane");
            let row_len = width as usize * 3;
            for (y, row) in plane
                .data
                .chunks_exact_mut(plane.stride)
                .take(height as usize)
                .enumerate()
            {
                let start = y * row_len;
                row[..row_len].copy_from_slice(&source.as_raw()[start..start + row_len]);
            }
        }

        let mut context = HeifContext::new().expect("heif context");
        context
            .encode_image(&heif_image, &mut encoder, None)
            .expect("encode heic fixture");
        Some(context.write_to_bytes().expect("serialize heic fixture"))
    }

    /// The HEIC path (D9), exercised through libheif in both directions.
    #[test]
    fn decodes_heic_through_libheif() {
        let (width, height) = (64u32, 32u32);
        let Some(bytes) = heic_fixture(width, height) else {
            eprintln!("skipping: this libheif has no HEVC encoder to build the fixture with");
            return;
        };

        let dir = ScratchDir::new("decode-heic");
        let path = dir.path().join("fixture.heic");
        std::fs::write(&path, bytes).expect("write heic fixture");

        let decoded = decode(&path, ImageFormat::Heic).expect("decode heic");
        assert_eq!(decoded.dimensions(), (width, height));

        // Lossless in, so the quadrants should come back recognizably — but
        // through a YCbCr round trip, hence "recognizably" rather than "equal".
        let [top_left, top_right, bottom_left, bottom_right] = corners(&decoded);
        assert!(top_left.0[0] > 200 && top_left.0[1] < 60, "{top_left:?}");
        assert!(top_right.0[1] > 200 && top_right.0[0] < 60, "{top_right:?}");
        assert!(
            bottom_left.0[2] > 200 && bottom_left.0[0] < 60,
            "{bottom_left:?}"
        );
        assert!(
            bottom_right.0[0] > 200 && bottom_right.0[1] > 200 && bottom_right.0[2] < 60,
            "{bottom_right:?}"
        );
    }

    /// **The double-rotation guard.**
    ///
    /// libheif applies the container's own orientation during decode. If
    /// `extract` also reported the EXIF orientation tag, the caller would
    /// rotate a second time and every iPhone portrait would come out face
    /// down — a bug that produces a plausible-looking image and so survives
    /// casual review.
    ///
    /// Asserting on the composition rather than on `decode` alone is the point:
    /// it is the pair that has to turn the photo exactly once.
    #[test]
    fn a_heic_is_never_rotated_twice() {
        let (width, height) = (64u32, 32u32);
        let Some(bytes) = heic_fixture(width, height) else {
            eprintln!("skipping: this libheif has no HEVC encoder to build the fixture with");
            return;
        };

        let dir = ScratchDir::new("heic-orientation");
        let path = dir.path().join("tagged.heic");
        std::fs::write(&path, bytes).expect("write heic fixture");

        // Give the container an EXIF orientation on top of pixels libheif has
        // already straightened — the shape of a file from a phone.
        exiftool(&[
            "-overwrite_original",
            "-Orientation#=6",
            path.to_str().expect("scratch path is utf-8"),
        ]);
        assert!(
            exiftool(&["-s", "-Orientation", path.to_str().expect("utf-8")]).contains("Rotate"),
            "fixture should carry an orientation tag"
        );

        assert_eq!(
            crate::photos::exif::extract(&path).orientation,
            None,
            "a HEIF container's orientation belongs to the decoder, not the caller"
        );

        // Which means the pipeline's decode step leaves the geometry alone —
        // exactly as libheif, heif-dec and every other libheif-based tool
        // render the same file.
        let decoded = testutil::decode_upright(&path, ImageFormat::Heic);
        assert_eq!(decoded.dimensions(), (width, height));
    }
}
