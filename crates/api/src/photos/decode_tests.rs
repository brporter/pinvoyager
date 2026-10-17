pub(crate) mod testutil {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    use image::{DynamicImage, Rgb, RgbImage};

    /// A unique temporary directory that removes itself.
    ///
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

    /// Asymmetric colored quadrants distinguish rotations from reflections.
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

    /// Runs exiftool and returns stdout. Fails the test if the tool is unavailable
    /// or exits unsuccessfully.
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
    let bytes =
        crate::photos::variants::encode_variant(&source, crate::photos::VariantKind::Thumb, 90.0)
            .expect("encode webp fixture");
    std::fs::write(&webp_path, bytes).expect("write webp fixture");
    let decoded = decode(&webp_path, ImageFormat::WebP).expect("decode webp");
    assert_eq!(decoded.dimensions(), (64, 32));
}

fn convert_plane(
    width: u32,
    height: u32,
    bits: u8,
    stride: usize,
    data: Option<&[u8]>,
) -> Result<DynamicImage, PhotoError> {
    image_from_interleaved(data.map(|data| Plane {
        width,
        height,
        storage_bits_per_pixel: bits,
        bits_per_pixel: bits,
        stride,
        data,
    }))
}

#[test]
fn heic_plane_conversion_handles_rgb_rgba_and_padded_rows() {
    let rgb = convert_plane(1, 2, 24, 5, Some(&[255, 0, 0, 9, 9, 0, 255, 0, 9, 9]))
        .expect("valid RGB plane");
    assert_eq!(rgb.color(), image::ColorType::Rgb8);
    assert_eq!(rgb.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(rgb.get_pixel(0, 1).0, [0, 255, 0, 255]);

    let rgba = convert_plane(1, 1, 32, 4, Some(&[1, 2, 3, 4])).expect("valid RGBA plane");
    assert_eq!(rgba.color(), image::ColorType::Rgba8);
    assert_eq!(rgba.get_pixel(0, 0).0, [1, 2, 3, 4]);
}

#[test]
fn heic_plane_conversion_rejects_missing_and_unusable_planes() {
    for (data, width, height, bits, stride) in [
        (None, 1, 1, 24, 3),
        (Some(&[][..]), 0, 1, 24, 3),
        (Some(&[][..]), 1, 1, 16, 2),
        (Some(&[][..]), 2, 1, 24, 5),
        (Some(&[1, 2, 3][..]), 1, 2, 24, 3),
    ] {
        assert!(matches!(
            convert_plane(width, height, bits, stride, data),
            Err(PhotoError::Undecodable(_))
        ));
    }
}

#[test]
fn image_format_mapping_is_explicit_for_each_sniffed_format() {
    assert_eq!(image_format(ImageFormat::Jpeg), image::ImageFormat::Jpeg);
    assert_eq!(image_format(ImageFormat::Png), image::ImageFormat::Png);
    assert_eq!(image_format(ImageFormat::WebP), image::ImageFormat::WebP);
    assert_eq!(image_format(ImageFormat::Heic), image::ImageFormat::Avif);
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

    assert!(matches!(
        decode(&missing, ImageFormat::Heic),
        Err(PhotoError::Storage(_))
    ));

    let malformed_heic = dir.path().join("malformed.heic");
    std::fs::write(&malformed_heic, b"not a HEIC container").expect("write malformed HEIC");
    assert!(matches!(
        decode(&malformed_heic, ImageFormat::Heic),
        Err(PhotoError::Undecodable(_))
    ));
}

/// Decode a committed synthetic fixture; running tests needs only a decoder.
#[test]
fn decodes_heic_through_libheif() {
    let (width, height) = (64u32, 32u32);
    let bytes = include_bytes!("../../tests/fixtures/quadrants-rgb.heic");

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

/// HEIC pipeline decoding does not apply EXIF orientation after libheif
/// has applied the container transformation.
#[test]
fn a_heic_is_never_rotated_twice() {
    let (width, height) = (64u32, 32u32);
    let bytes = include_bytes!("../../tests/fixtures/quadrants-rgb.heic");

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

#[test]
fn heic_alpha_survives_native_decoding() {
    let dir = ScratchDir::new("heic-alpha");
    let path = dir.path().join("alpha.heic");
    std::fs::write(
        &path,
        include_bytes!("../../tests/fixtures/quadrants-rgba.heic"),
    )
    .expect("fixture");
    let decoded = decode(&path, ImageFormat::Heic).expect("decode RGBA HEIC");
    assert_eq!(decoded.dimensions(), (64, 32));
    assert_eq!(decoded.color(), image::ColorType::Rgba8);
    assert_eq!(decoded.get_pixel(4, 4).0[3], 64);
    assert_eq!(decoded.get_pixel(60, 4).0[3], 255);
}
