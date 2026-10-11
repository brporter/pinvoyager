# Image regression fixtures

`upload-regression.heic` is a synthetic 64 × 48 RGB image: the left half is blue
and the right half is orange. It contains no user data. It was encoded with
libheif 1.23.5 using:

```sh
heif-enc -q 90 -o upload-regression.heic source.png
```

Tests read the committed fixture, so they require the application's native HEVC decoder
but no encoder. The upload test must fail if the decoder is missing.

`crates/api/src/http/photos_tests.rs` exercises the fixture in
`heic_upload_generates_and_serves_every_variant`,
`corrupt_heic_upload_is_rejected_without_a_photo_or_job`, and
`heic_conversion_failure_marks_the_photo_failed`. They check ingestion, voyage
ownership, pending/ready/failed statuses, every served WebP variant, truncated
input rejection, and corruption between ingestion and worker processing.

`quadrants-rgb.heic` and `quadrants-rgba.heic` are synthetic 64 × 32 images
with red, green, blue, and yellow quadrants (clockwise from the upper left:
red, green, yellow, blue). The RGBA fixture has alpha 64 on its left half and
255 on its right half. They contain no user data and were encoded with
libheif 1.23.5 using `heif-enc -q 100` from 8-bit RGB/RGBA PNGs.
`photos/decode_tests.rs` checks orientation, quadrant colors, and preserved
transparency without requiring an HEVC encoder on the test machine.
