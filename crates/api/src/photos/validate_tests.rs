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

/// A `.jpg` whose bytes are
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

/// The HEIC path, end to end: a file libheif wrote, sniffed from its magic
/// bytes and then probed for dimensions through libheif's container parse.
/// Both halves matter — an `ftyp` brand this module does not recognize
/// would make the probe unreachable no matter how correct it is.
#[test]
fn dimensions_are_read_from_a_real_heic() {
    let dir = scratch_dir("heic");
    let path = dir.join("probe.heic");

    std::fs::write(
        &path,
        include_bytes!("../../tests/fixtures/quadrants-rgb.heic"),
    )
    .expect("write committed fixture");

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
        (64, 32)
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

#[cfg(unix)]
#[test]
fn heic_probe_rejects_non_utf8_paths_without_a_lossy_conversion() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));
    assert!(matches!(probe_heic(&path), Err(PhotoError::Undecodable(_))));
}
