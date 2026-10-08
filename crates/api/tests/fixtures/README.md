# Image regression fixtures

`upload-regression.heic` is a synthetic 64 × 48 RGB image: the left half is blue
and the right half is orange. It contains no user data. It was encoded with
libheif 1.23.5 using:

```sh
heif-enc -q 90 -o upload-regression.heic source.png
```

Keep this committed fixture rather than encoding it during tests. An encoder is
not needed to run the regression; the application's native HEVC decoder is. A
missing decoder must fail the upload test, rather than silently skipping it.

`crates/api/src/http/photos_tests.rs` exercises the fixture in
`heic_upload_generates_and_serves_every_variant`,
`corrupt_heic_upload_is_rejected_without_a_photo_or_job`, and
`heic_conversion_failure_marks_the_photo_failed`. They check ingestion, voyage
ownership, pending/ready/failed statuses, every served WebP variant, truncated
input rejection, and corruption between ingestion and worker processing.
