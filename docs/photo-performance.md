# On-demand photo performance regression

`crates/api/tests/photo_performance.rs::large_photos_report_resize_and_encode_spans`
measures the production background worker using its OpenTelemetry `photo.resize` and
`photo.encode` spans. It is marked `#[ignore]`: ordinary `cargo test` and coverage runs
do not generate large images or run this benchmark. Tests of span attribution and timing
comparisons run in the ordinary suite.

## What it exercises

The test creates an isolated PostgreSQL database through `#[sqlx::test]`, stages
original files in temporary storage, creates fresh `pending` photo rows, and
enqueues them through `photos::worker::spawn` / `VariantQueue::enqueue`. The
production worker performs EXIF reading, decode, orientation, all three variant
resizes and WebP encodes, atomic writes, and the database transition to `ready`.
It retains the configured worker concurrency and WebP quality. HTTP upload,
authentication, publication, browser polling, and network transfer are outside
this worker benchmark.

There is one unmeasured warm-up batch followed by three measured batches. Each
batch contains all three fixtures; jobs run concurrently under the same worker
semaphore used by the API. Every iteration creates new rows so the worker's
already-ready shortcut cannot produce a false speedup. Variant files are
overwritten as in a normal recovery job. The startup recovery sweep finishes
before any photo rows exist, avoiding duplicate recovery/live jobs.

An in-memory OpenTelemetry exporter captures completed spans. The helper
`tests/support/photo_performance.rs::photo_samples` walks the actual
`photo.process <UUID>` → `photo.generate` → `photo.variant` parent relationships,
including trace IDs, to assign each resize and encode to its photo and variant.
It requires exactly one resize and one encode per variant and rejects missing,
duplicate, failed-process, zero-duration, or backwards-duration observations.
The test also verifies database readiness, decodes all nine output WebPs per
batch, and checks dimensions and alpha preservation. Output verification happens
after the measured batch finishes.

The report contains 54 measured phase samples: three fixtures × three variants ×
two phases × three iterations. Each of the 18 fixture/variant/phase groups has
its own median, minimum, and maximum. Raw samples retain photo IDs and trace IDs.
Durations are OpenTelemetry end time minus start time, not surrounding test
timers. Batch wall time is reported separately and includes queue/DB work and
the completion polling interval of up to 10 ms. Concurrent span durations must
not be added together and described as batch elapsed time.

Collector and Jaeger are unnecessary for this test. Captured benchmark spans
stay in memory; they are not sent to the development collector or mixed into
interactive traces.

## Large fixtures

| Fixture | Dimensions | Input |
|---|---|---|
| `jpeg-12mp` | 4032 × 3024 | RGB JPEG, quality 90 |
| `heic-24mp` | 5712 × 4284 | RGB HEIC, quality 82 |
| `png-alpha-24mp` | 4284 × 5712 | RGBA PNG with translucent regions |

`photo_performance.rs::pixels` generates repeatable gradients, textured tiles,
and fine edges. These are representative pixel counts, orientations, and pixel
formats, not a corpus of natural photographs; content-dependent encoding costs
may differ for real photos. No private uploads or external image downloads are
used. Generated originals are checked against the normal upload byte limit.

Fixture preparation is outside measurement. Files are cached in
`target/photo-perf/fixtures/v1/`, so both compiler profiles process **identical
compressed bytes**, including identical HEIC encoder output. Reports include
dimensions, byte sizes, and SHA-256 hashes. Baseline comparison rejects a changed
fixture recipe or changed bytes. If the recipe changes, increment
`FIXTURE_VERSION` and the cache directory version, then collect fresh baselines.

First-time HEIC fixture generation requires a libheif HEVC **encoder** as well as
the application's decoder. The test fails with an explicit message if it is
missing; HEIC is never silently skipped. The encoder uses an `ultrafast` preset
when exposed by the installed plugin. Later runs reuse the cached HEIC. The
native dependencies from [running locally](running.md) are otherwise sufficient.

## Run the comparison

Start PostgreSQL and set the test database URL. Stop other CPU-heavy jobs and
do not run the two profiles simultaneously. Run from the repository root:

```sh
docker compose up -d db
export DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager'
export SQLX_OFFLINE=true

PHOTO_PERF_BUILD_LABEL=unoptimized-test \
PHOTO_PERF_OUTPUT="$PWD/target/photo-perf/unoptimized.json" \
cargo test --locked --test photo_performance -- \
  --ignored --exact large_photos_report_resize_and_encode_spans \
  --nocapture --test-threads=1

RUSTFLAGS='-C target-cpu=native' TARGET_CPU=native \
PHOTO_PERF_BUILD_LABEL=photo-perf-native \
PHOTO_PERF_OUTPUT="$PWD/target/photo-perf/optimized.json" \
PHOTO_PERF_BASELINE="$PWD/target/photo-perf/unoptimized.json" \
cargo test --locked --profile photo-perf --test photo_performance -- \
  --ignored --exact large_photos_report_resize_and_encode_spans \
  --nocapture --test-threads=1
```

The first benchmark command uses the `test` profile, which inherits the unoptimized
`dev` profile. The opt-in `photo-perf` profile in root `Cargo.toml` inherits `release`,
uses optimization level 3, full Rust link-time optimization, and one code generation
unit. `RUSTFLAGS` enables native CPU tuning for Rust; `TARGET_CPU` asks the bundled
libwebp build to use its supported native compiler architecture flag. Its C compilation
also inherits optimization level 3. This does not enable cross-language LTO or
profile-guided optimization.

The profile sets `strip = "none"` to avoid the macOS/LLVM proc-macro loading
issue described in [Rust issue 157750](https://github.com/rust-lang/rust/issues/157750).
This leaves runtime optimization enabled.

Both runs use the same resize filter, image sizes, quality, queue capacity, and worker
concurrency. Compiler profile semantics are described in the [Cargo profile
reference](https://doc.rust-lang.org/cargo/reference/profiles.html) and native Rust CPU
tuning in the [rustc
reference](https://doc.rust-lang.org/rustc/codegen-options/index.html#target-cpu).

For meaningful comparisons, unset unrelated compiler/profile overrides, use the
same machine and native libraries, and keep the process configuration constant.
Reports record the caller-supplied build label, compiler version, platform,
available CPU count, relevant environment flags, fixture hashes, worker
concurrency, quality, and output sizes. A label describes the command you ran;
it does not independently prove the compiler flags. Cargo's build output records
the selected profile.

## Detect later regressions

After reviewing the optimized result, keep that report as the baseline. Repeat
the same optimized command with `PHOTO_PERF_BASELINE` pointing at it and
`PHOTO_PERF_OUTPUT` pointing at a **different** file. A baseline is optional for
the initial measurement, but required to assert a timing regression.

`performance::compare` fails the test if **any** fixture/variant/phase median
exceeds `baseline_median × 1.20 + 2 ms`. The default 20% tolerance can be changed
with `PHOTO_PERF_MAX_REGRESSION_PERCENT`; negative and non-finite tolerances are
rejected. The 2 ms allowance avoids failures from tiny timer/scheduling changes.
The current report is written before the timing comparison, including when a
regression fails, so the slow phase can be inspected.

The comparison also refuses different metric sets, invalid durations, changed
fixture bytes, sample counts, concurrency, quality, or variant dimensions. Use
separate baselines for different machines and build profiles. The deliberate
unoptimized → optimized comparison above measures compiler gains; routine
regression checks should compare the same profile.

Three samples provide a practical local check, not a statistically rigorous tail
latency study. Investigate a failure and rerun under comparable load before
accepting a replacement baseline. Do not collect timing baselines under coverage
instrumentation.

## Tests of timing measurements and comparisons

```sh
SQLX_OFFLINE=true cargo test --test photo_performance_contract
SQLX_OFFLINE=true cargo test --test photo_performance
```

The first checks parent/variant attribution, missing and duplicate spans, invalid
durations, median calculation, and regression sensitivity. The second reports the
large-image test as ignored unless explicitly requested. Reports and generated
fixtures live under `target/photo-perf/`; no machine-specific timing baseline is
committed. See [testing](testing.md) for the ordinary suite and coverage commands.
