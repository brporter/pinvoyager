use super::*;

use image::GenericImageView as _;

use crate::photos::decode::testutil::{
    ScratchDir, asymmetric_image, decode_upright, exiftool, write_jpeg,
};

const QUALITY: f32 = 82.0;

/// Tags that must never appear on a served file. Each is one that a real
/// camera writes and that D6 says a public artifact must not carry.
const FORBIDDEN: [&str; 9] = [
    "GPSLatitude",
    "GPSLongitude",
    "GPSPosition",
    "DateTimeOriginal",
    "CreateDate",
    "Make",
    "Model",
    "Orientation",
    "SerialNumber",
];

/// Splits a WebP file into its RIFF chunk identifiers.
///
/// The container is the only place metadata could be hiding, so reading the
/// chunk list directly is a check that does not depend on exiftool knowing
/// where to look.
fn riff_chunks(bytes: &[u8]) -> Vec<String> {
    assert!(bytes.len() >= 12, "too short to be a RIFF file");
    assert_eq!(&bytes[0..4], b"RIFF", "not a RIFF container");
    assert_eq!(&bytes[8..12], b"WEBP", "not a WebP file");

    let mut chunks = Vec::new();
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let id = String::from_utf8_lossy(&bytes[offset..offset + 4]).into_owned();
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        chunks.push(id);
        // Chunks are padded to an even length.
        offset += 8 + size + (size & 1);
    }
    chunks
}

/// Builds an original that is as loaded with metadata as a real photograph:
/// GPS in the southern-western quadrant, a capture time, a camera, a serial
/// number, and a portrait orientation tag.
fn photo_with_everything(dir: &ScratchDir, name: &str) -> std::path::PathBuf {
    let path = write_jpeg(dir, name, 1600, 900);
    exiftool(&[
        "-overwrite_original",
        "-GPSLatitude=34 36 12.0",
        "-GPSLatitudeRef=S",
        "-GPSLongitude=58 22 54.0",
        "-GPSLongitudeRef=W",
        "-GPSAltitude=25",
        "-DateTimeOriginal=2019:03:14 09:26:53",
        "-CreateDate=2019:03:14 09:26:53",
        "-Make=PinVoyager",
        "-Model=Test Camera 1",
        "-SerialNumber=SN-0000-SECRET",
        "-Artist=A Person",
        "-XMP:Description=a description that must not survive",
        "-Orientation#=6",
        path.to_str().expect("scratch path is utf-8"),
    ]);
    path
}

/// Every public image must strip original metadata.
///
/// Every variant of a photograph that carried GPS, a timestamp, a camera
/// identity and an XMP packet is run through exiftool, and none of it may
/// come back out. Re-encoding is assumed to strip; this is the assertion
/// that it actually did.
#[test]
fn exiftool_finds_no_metadata_on_any_generated_variant() {
    let dir = ScratchDir::new("variant-strip");
    let original = photo_with_everything(&dir, "loaded");

    // Sanity: the fixture really does carry what we claim, otherwise this
    // test would pass against an empty file and prove nothing.
    let before = exiftool(&[
        "-s",
        "-GPSLatitude",
        "-Make",
        "-DateTimeOriginal",
        original.to_str().expect("scratch path is utf-8"),
    ]);
    for tag in ["GPSLatitude", "Make", "DateTimeOriginal"] {
        assert!(
            before.contains(tag),
            "fixture is missing {tag}; the test would prove nothing"
        );
    }

    let decoded = decode_upright(&original, crate::photos::ImageFormat::Jpeg);

    for kind in VariantKind::ALL {
        let bytes = encode_variant(&decoded, kind, QUALITY).expect("encode variant");
        let path = dir.path().join(format!("{}.webp", kind.as_str()));
        std::fs::write(&path, &bytes).expect("write variant");
        let file = path.to_str().expect("scratch path is utf-8");

        // Ask exiftool for every metadata family a camera or an editor
        // could have written. Anything at all in the output is a leak.
        let metadata = exiftool(&[
            "-s",
            "-G1",
            "-EXIF:all",
            "-GPS:all",
            "-XMP:all",
            "-IPTC:all",
            "-ICC_Profile:all",
            "-Photoshop:all",
            "-MakerNotes:all",
            file,
        ]);
        assert!(
            metadata.trim().is_empty(),
            "{} variant carries metadata:\n{metadata}",
            kind.as_str()
        );

        // And a belt-and-braces pass over everything exiftool can see,
        // ignoring only the groups that describe the file on disk rather
        // than its contents.
        let everything = exiftool(&["-a", "-u", "-s", "-G1", file]);
        for line in everything.lines() {
            let describes_the_file_itself = line.starts_with("[File]")
                || line.starts_with("[System]")
                || line.starts_with("[ExifTool]");
            if describes_the_file_itself {
                continue;
            }
            for tag in FORBIDDEN {
                assert!(
                    !line.contains(tag),
                    "{} variant leaks {tag}: {line}",
                    kind.as_str()
                );
            }
        }

        // The container itself, read without exiftool's help: no EXIF, XMP
        // or color-profile chunk may exist at all.
        let chunks = riff_chunks(&bytes);
        for chunk in &chunks {
            assert!(
                !matches!(chunk.as_str(), "EXIF" | "XMP " | "ICCP"),
                "{} variant contains a {chunk} chunk: {chunks:?}",
                kind.as_str()
            );
        }
    }
}

/// D9's other half: the variant must be *lossy* WebP. A lossless encode of
/// a photograph is routinely larger than the JPEG it came from, which would
/// invert the reason for choosing WebP.
#[test]
fn variants_are_lossy_webp() {
    let image = asymmetric_image(800, 600);
    let bytes = encode_variant(&image, VariantKind::Display, QUALITY).expect("encode");

    let chunks = riff_chunks(&bytes);
    assert!(
        chunks.iter().any(|c| c == "VP8 "),
        "expected a lossy VP8 bitstream, got {chunks:?}"
    );
    assert!(
        !chunks.iter().any(|c| c == "VP8L"),
        "expected lossy, got a lossless VP8L bitstream: {chunks:?}"
    );
}

/// A photo tagged portrait must *be* portrait once it is a variant, and it
/// must get there with no orientation tag to lean on — because there is no
/// metadata left to carry one.
#[test]
fn a_rotated_photo_comes_out_upright_with_no_orientation_tag() {
    let dir = ScratchDir::new("variant-orientation");
    let path = write_jpeg(&dir, "portrait", 1600, 900);
    exiftool(&[
        "-overwrite_original",
        "-Orientation#=6",
        path.to_str().expect("scratch path is utf-8"),
    ]);

    let decoded = decode_upright(&path, crate::photos::ImageFormat::Jpeg);
    assert_eq!(
        decoded.dimensions(),
        (900, 1600),
        "the decode step must rotate"
    );

    let bytes = encode_variant(&decoded, VariantKind::Display, QUALITY).expect("encode");
    let out = dir.path().join("portrait.webp");
    std::fs::write(&out, &bytes).expect("write variant");
    let file = out.to_str().expect("scratch path is utf-8");

    // The pixels are portrait...
    let sizes = exiftool(&["-s", "-ImageWidth", "-ImageHeight", file]);
    assert!(
        sizes.contains("675"),
        "expected a 675 px width, got {sizes}"
    );
    assert!(
        sizes.contains("1200"),
        "expected a 1200 px height, got {sizes}"
    );

    // ...and there is no orientation tag telling a viewer to turn them.
    let orientation = exiftool(&["-s", "-Orientation", file]);
    assert!(
        orientation.trim().is_empty(),
        "variant still carries an orientation tag: {orientation}"
    );
}

#[test]
fn resizing_preserves_the_aspect_ratio_and_caps_the_long_edge() {
    let landscape = asymmetric_image(4000, 3000);
    let resized = fit_within(&landscape, 1200);
    assert_eq!(resized.as_ref().dimensions(), (1200, 900));

    let portrait = asymmetric_image(3000, 4000);
    let resized = fit_within(&portrait, 1200);
    assert_eq!(resized.as_ref().dimensions(), (900, 1200));

    // A square lands on the cap in both axes.
    let square = asymmetric_image(2000, 2000);
    assert_eq!(fit_within(&square, 400).as_ref().dimensions(), (400, 400));
}

/// Never enlarge: a 200 px scan blown up to 2560 px is a bigger file that
/// looks worse than the original.
#[test]
fn a_small_original_is_never_enlarged() {
    let small = asymmetric_image(320, 240);

    for kind in VariantKind::ALL {
        let fitted = fit_within(&small, kind.max_edge());
        assert_eq!(
            fitted.as_ref().dimensions(),
            (320, 240),
            "{} enlarged a small original",
            kind.as_str()
        );
        // And the no-resize path must not copy the buffer.
        assert!(
            matches!(fitted, Cow::Borrowed(_)),
            "{} copied a buffer it did not need to resize",
            kind.as_str()
        );
    }

    // The encoded variant is the original size too, not the cap.
    let bytes = encode_variant(&small, VariantKind::Full, QUALITY).expect("encode");
    let decoded = image::load_from_memory(&bytes).expect("decode variant");
    assert_eq!(decoded.dimensions(), (320, 240));
}

#[test]
fn each_kind_caps_at_its_own_edge() {
    let source = asymmetric_image(3000, 1500);

    for (kind, expected) in [
        (VariantKind::Thumb, (400, 200)),
        (VariantKind::Display, (1200, 600)),
        (VariantKind::Full, (2560, 1280)),
    ] {
        let bytes = encode_variant(&source, kind, QUALITY).expect("encode");
        let decoded = image::load_from_memory(&bytes).expect("decode variant");
        assert_eq!(decoded.dimensions(), expected, "{}", kind.as_str());
    }
}

/// An image with transparency keeps it; an opaque one is not padded out to
/// four channels for nothing.
#[test]
fn alpha_survives_and_is_not_invented() {
    let mut translucent = image::RgbaImage::new(64, 64);
    for (x, _y, pixel) in translucent.enumerate_pixels_mut() {
        *pixel = image::Rgba([255, 0, 0, if x < 32 { 0 } else { 255 }]);
    }
    let bytes = encode_variant(
        &DynamicImage::ImageRgba8(translucent),
        VariantKind::Thumb,
        QUALITY,
    )
    .expect("encode rgba");
    let decoded = image::load_from_memory(&bytes).expect("decode variant");
    assert_eq!(decoded.color(), image::ColorType::Rgba8);
    assert_eq!(decoded.get_pixel(4, 4).0[3], 0, "transparency was lost");

    // Alpha pushes libwebp into the extended container (VP8X + ALPH), which
    // is the form that *can* hold metadata chunks. It must still hold none.
    let chunks = riff_chunks(&bytes);
    for chunk in &chunks {
        assert!(
            !matches!(chunk.as_str(), "EXIF" | "XMP " | "ICCP"),
            "the extended container gained a {chunk} chunk: {chunks:?}"
        );
    }

    let opaque = asymmetric_image(64, 64);
    let bytes = encode_variant(&opaque, VariantKind::Thumb, QUALITY).expect("encode rgb");
    let decoded = image::load_from_memory(&bytes).expect("decode variant");
    assert_eq!(
        decoded.color(),
        image::ColorType::Rgb8,
        "an opaque photo should not gain an alpha channel"
    );
}

#[test]
fn an_empty_image_is_rejected_before_calling_libwebp() {
    let empty = DynamicImage::ImageRgb8(image::RgbImage::new(0, 1));

    assert!(matches!(
        encode_variant(&empty, VariantKind::Thumb, QUALITY),
        Err(PhotoError::Unencodable(message)) if message == "cannot encode a 0x1 image"
    ));
}

/// A misconfigured quality must not make photos unviewable.
#[test]
fn out_of_range_quality_is_clamped_rather_than_fatal() {
    let source = asymmetric_image(64, 64);

    for quality in [-50.0, 0.0, 100.0, 1_000.0] {
        let bytes =
            encode_variant(&source, VariantKind::Thumb, quality).expect("encode at {quality}");
        assert!(!bytes.is_empty());
        image::load_from_memory(&bytes).expect("variant decodes");
    }
}

#[test]
fn variant_kinds_round_trip_through_their_path_segment() {
    for kind in VariantKind::ALL {
        assert_eq!(VariantKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(VariantKind::parse("original"), None);
    assert_eq!(VariantKind::parse("../../etc/passwd"), None);
}

#[test]
fn native_encoder_rejection_is_a_photo_error() {
    // VP8 permits at most 16383 pixels per dimension, even when total memory
    // is small. A native rejection must not panic on the worker thread.
    let image = DynamicImage::new_rgb8(16384, 1);
    assert!(matches!(
        encode_webp(&image, QUALITY),
        Err(PhotoError::Unencodable(_))
    ));
}
