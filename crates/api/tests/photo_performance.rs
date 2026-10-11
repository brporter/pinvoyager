//! On-demand end-to-end worker benchmark. See docs/photo-performance.md.
//! The ordinary test harness ignores this test; no large fixtures are generated
//! and no database is opened unless it is explicitly selected with --ignored.
#[path = "support/photo_performance.rs"]
mod performance;
#[path = "support/photo_voyage.rs"]
mod photo_voyage;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use image::{DynamicImage, Rgb, RgbImage};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use pinvoyager_api::{
    Config,
    db::repo::{
        photos::{self, NewPhoto},
        users,
    },
    photos::{
        ImageFormat, VariantKind,
        storage::{self, ContentHash},
        validate,
        worker::{self, VariantJob},
    },
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

const ITERATIONS: usize = 3;
const FIXTURE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Fixture {
    name: String,
    format: String,
    width: u32,
    height: u32,
    sha256: String,
    bytes: u64,
}

#[derive(Serialize, Deserialize)]
struct Report {
    schema_version: u32,
    fixture_version: u32,
    build_label: String,
    rustc: String,
    platform: String,
    build_environment: BTreeMap<String, String>,
    logical_cpus: usize,
    variant_concurrency: usize,
    webp_quality: f32,
    variant_max_edges: Vec<u32>,
    warmup_batches: usize,
    measured_batches: usize,
    fixtures: Vec<Fixture>,
    batch_wall_ms: Vec<f64>,
    samples: Vec<performance::Sample>,
    summary: BTreeMap<String, performance::Summary>,
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Deterministic gradients, textured tiles, and fine edges. Synthetic inputs
/// reproduce pixel counts and RGB/RGBA work without redistributing user photos.
fn pixels(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        let tile = (x / 32).wrapping_mul(2654435761) ^ (y / 32).wrapping_mul(2246822519);
        let detail = ((x * 13 + y * 17) % 17) as i32 - 8;
        let channel = |shift| ((tile.wrapping_shr(shift) % 80) as i32 + detail).max(0) as u8;
        Rgb([
            ((x * 160 / width) as u8).saturating_add(channel(0)),
            ((y * 160 / height) as u8).saturating_add(channel(8)),
            (((x + y) * 120 / (width + height)) as u8).saturating_add(channel(16)),
        ])
    })
}

fn write_heic(path: &Path, rgb: &RgbImage) {
    use libheif_rs::{
        Channel, ColorSpace, CompressionFormat, EncoderParameterValue, EncoderQuality, HeifContext,
        Image, LibHeif, RgbChroma,
    };
    let lib = LibHeif::new();
    let mut encoder = lib.encoder_for_format(CompressionFormat::Hevc)
        .expect("fixture setup needs a libheif HEVC encoder (brew install libheif); never silently skip HEIC");
    encoder
        .set_quality(EncoderQuality::Lossy(82))
        .expect("HEIC fixture quality");
    // x265's faster fixture generation is outside every timed span. Other
    // plugins can retain their defaults; the report pins the resulting bytes.
    if encoder.parameters_names().iter().any(|p| p == "preset") {
        encoder
            .set_parameter_value("preset", EncoderParameterValue::String("ultrafast".into()))
            .expect("HEVC preset");
    }
    let (width, height) = rgb.dimensions();
    let mut image = Image::new(width, height, ColorSpace::Rgb(RgbChroma::Rgb)).expect("HEIC image");
    image
        .create_plane(Channel::Interleaved, width, height, 8)
        .expect("HEIC plane");
    {
        let mut planes = image.planes_mut();
        let plane = planes.interleaved.as_mut().expect("RGB plane");
        for (row, source) in plane
            .data
            .chunks_exact_mut(plane.stride)
            .take(height as usize)
            .zip(rgb.as_raw().chunks_exact(width as usize * 3))
        {
            row[..source.len()].copy_from_slice(source);
        }
    }
    let mut context = HeifContext::new().expect("HEIC context");
    context
        .encode_image(&image, &mut encoder, None)
        .expect("HEIC fixture encode");
    fs::write(path, context.write_to_bytes().expect("HEIC bytes")).expect("HEIC fixture file");
}

fn fixtures(directory: &Path, config: &Config) -> Vec<Fixture> {
    fs::create_dir_all(directory).expect("fixture cache directory");
    [
        ("jpeg-12mp", 4032, 3024, ImageFormat::Jpeg),
        ("heic-24mp", 5712, 4284, ImageFormat::Heic),
        ("png-alpha-24mp", 4284, 5712, ImageFormat::Png),
    ]
    .into_iter()
    .map(|(name, width, height, format)| {
        let path = directory.join(name);
        if !path.exists() {
            eprintln!("Generating {name} ({width}x{height}); setup is not timed");
            let temp = directory.join(format!("{name}-{}.tmp", Uuid::now_v7()));
            let rgb = pixels(width, height);
            match format {
                ImageFormat::Jpeg => image::codecs::jpeg::JpegEncoder::new_with_quality(
                    fs::File::create(&temp).expect("JPEG file"),
                    90,
                )
                .encode_image(&rgb)
                .expect("JPEG fixture"),
                ImageFormat::Heic => write_heic(&temp, &rgb),
                ImageFormat::Png => {
                    let mut rgba = DynamicImage::ImageRgb8(rgb).into_rgba8();
                    for (x, y, p) in rgba.enumerate_pixels_mut() {
                        p[3] = if (x / 128 + y / 128) % 3 == 0 {
                            160
                        } else {
                            255
                        };
                    }
                    rgba.save_with_format(&temp, image::ImageFormat::Png)
                        .expect("PNG fixture");
                }
                ImageFormat::WebP => unreachable!("no WebP source fixture"),
            }
            fs::rename(temp, &path).expect("commit complete fixture");
        }
        assert_eq!(
            validate::probe_dimensions(&path, format).expect("fixture dimensions"),
            (width, height)
        );
        let bytes = fs::metadata(&path).expect("fixture metadata").len();
        assert!(
            bytes <= config.photos.max_upload_bytes,
            "{name} must fit the upload limit"
        );
        Fixture {
            name: name.into(),
            format: format.as_str().into(),
            width,
            height,
            bytes,
            sha256: ContentHash::of_file(&path)
                .expect("fixture hash")
                .as_str()
                .to_owned(),
        }
    })
    .collect()
}

fn verify_variants(root: &Path, fixture: &Fixture) {
    let hash = ContentHash::try_from(fixture.sha256.clone()).expect("valid fixture hash");
    for kind in VariantKind::ALL {
        let path = storage::variant_path(root, &hash, kind);
        let decoded = image::open(path).expect("a successfully processed variant must decode");
        let edge = kind.max_edge();
        let expected = if fixture.width >= fixture.height {
            (
                edge,
                (u64::from(fixture.height) * u64::from(edge) / u64::from(fixture.width)) as u32,
            )
        } else {
            (
                (u64::from(fixture.width) * u64::from(edge) / u64::from(fixture.height)) as u32,
                edge,
            )
        };
        assert_eq!((decoded.width(), decoded.height()), expected);
        assert_eq!(
            decoded.color().has_alpha(),
            fixture.name == "png-alpha-24mp"
        );
    }
}

#[sqlx::test]
#[ignore = "large-image performance regression; run explicitly as documented in docs/photo-performance.md"]
async fn large_photos_report_resize_and_encode_spans(pool: PgPool) {
    let output = PathBuf::from(
        std::env::var("PHOTO_PERF_OUTPUT").expect("set PHOTO_PERF_OUTPUT to a report JSON path"),
    );
    let baseline_path = std::env::var("PHOTO_PERF_BASELINE").ok().map(PathBuf::from);
    assert_ne!(
        baseline_path.as_ref(),
        Some(&output),
        "never overwrite the baseline with this run"
    );
    if let Some(baseline) = &baseline_path
        && output.exists()
    {
        assert_ne!(
            fs::canonicalize(baseline).expect("baseline path"),
            fs::canonicalize(&output).expect("output path"),
            "baseline and output resolve to the same file"
        );
    }
    let baseline: Option<Report> = baseline_path.as_ref().map(|p| {
        serde_json::from_slice(&fs::read(p).expect("read baseline")).expect("baseline JSON")
    });
    let tolerance = std::env::var("PHOTO_PERF_MAX_REGRESSION_PERCENT")
        .unwrap_or_else(|_| "20".into())
        .parse::<f64>()
        .expect("numeric regression tolerance");
    assert!(tolerance.is_finite() && tolerance >= 0.0);
    let build_label = std::env::var("PHOTO_PERF_BUILD_LABEL")
        .expect("set PHOTO_PERF_BUILD_LABEL to describe the compiler settings");
    let scratch = Scratch(std::env::temp_dir().join(format!("pv-photo-perf-{}", Uuid::now_v7())));
    let mut config = Config::load().expect("config");
    config.photos.storage_root = scratch.0.clone();
    let config = Arc::new(config);
    let fixture_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/photo-perf/fixtures/v1");
    let inputs = {
        let directory = fixture_dir.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || fixtures(&directory, &config))
            .await
            .expect("fixture setup")
    };
    if let Some(baseline) = &baseline {
        assert_eq!(baseline.schema_version, 1, "baseline schema");
        assert_eq!(
            baseline.fixture_version, FIXTURE_VERSION,
            "fixture recipe changed"
        );
        assert_eq!(
            baseline.fixtures, inputs,
            "use identical fixture bytes for comparisons"
        );
        assert_eq!(
            baseline.variant_concurrency, config.photos.variant_concurrency,
            "worker concurrency changed"
        );
        assert_eq!(
            baseline.webp_quality, config.photos.webp_quality,
            "WebP quality changed"
        );
        assert_eq!(
            baseline.variant_max_edges,
            VariantKind::ALL.map(|k| k.max_edge()).to_vec(),
            "variant sizes changed"
        );
        assert_eq!(
            baseline.measured_batches, ITERATIONS,
            "sample counts changed"
        );
    }
    for input in &inputs {
        let destination = storage::original_path(
            &scratch.0,
            &ContentHash::try_from(input.sha256.clone()).expect("valid fixture hash"),
        );
        tokio::fs::create_dir_all(destination.parent().expect("original directory"))
            .await
            .expect("storage directory");
        tokio::fs::copy(fixture_dir.join(&input.name), destination)
            .await
            .expect("stage original");
    }
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    tracing_subscriber::registry()
        .with(
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("photo-performance"))
                .with_filter(pinvoyager_api::telemetry::SpanOnly),
        )
        .try_init()
        .expect("isolated benchmark subscriber");
    let queue = worker::spawn(pool.clone(), config.clone());
    // Recovery must finish before live rows exist, avoiding duplicate startup
    // work and making the measured job population exact.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !exporter
            .get_finished_spans()
            .expect("spans")
            .iter()
            .any(|s| s.name == "photo.recover")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup recovery completes");
    let owner = users::create(&pool, Some("Performance fixture"), None)
        .await
        .expect("owner");
    let mut samples = Vec::new();
    let mut batch_wall_ms = Vec::new();
    for iteration in 0..=ITERATIONS {
        exporter.reset();
        let voyage = photo_voyage::fixture_voyage(&pool, owner.id).await;
        let mut jobs = Vec::new();
        // Every iteration uses fresh pending rows; no already-ready shortcut.
        for input in &inputs {
            let photo = photos::create(
                &pool,
                &NewPhoto {
                    voyage_id: voyage,
                    owner_id: owner.id,
                    content_hash: input.sha256.clone(),
                    byte_size: input.bytes as i64,
                    width: input.width as i32,
                    height: input.height as i32,
                    original_format: input.format.clone(),
                    captured_at: None,
                    latitude: None,
                    longitude: None,
                },
            )
            .await
            .expect("pending photo");
            assert_eq!(photo.variant_status, worker::STATUS_PENDING);
            jobs.push(photo.id);
        }
        let batch = tracing::info_span!("photo.performance.batch", iteration);
        let started = Instant::now();
        for id in &jobs {
            let job = batch.in_scope(|| VariantJob::new(*id).with_voyage(voyage));
            queue.enqueue(job).await.expect("enqueue real worker job");
        }
        tokio::time::timeout(Duration::from_secs(600), async {
            loop {
                let finished = exporter.get_finished_spans().expect("spans");
                if jobs.iter().all(|id| {
                    finished
                        .iter()
                        .any(|s| s.name == format!("photo.process {id}"))
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("large photo batch completes within ten minutes");
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        drop(batch);
        provider.force_flush().expect("flush completed spans");
        let spans = exporter.get_finished_spans().expect("exported spans");
        for (input, id) in inputs.iter().zip(jobs) {
            let photo = photos::find_by_id(&pool, id)
                .await
                .expect("photo status")
                .expect("photo exists");
            assert_eq!(photo.variant_status, worker::STATUS_READY);
            let measured =
                performance::photo_samples(&spans, &id.to_string(), &input.name, iteration)
                    .expect("all six completed phase spans per photo");
            if iteration != 0 {
                samples.extend(measured);
            }
            let root = scratch.0.clone();
            let input = input.clone();
            tokio::task::spawn_blocking(move || verify_variants(&root, &input))
                .await
                .expect("verify output images");
        }
        eprintln!(
            "batch {iteration}: {wall_ms:.1} ms{}",
            if iteration == 0 {
                " (warm-up excluded)"
            } else {
                ""
            }
        );
        if iteration != 0 {
            batch_wall_ms.push(wall_ms);
        }
    }
    let summary = performance::summarize(&samples).expect("phase summary");
    assert_eq!(summary.len(), inputs.len() * 6);
    assert!(summary.values().all(|s| s.samples == ITERATIONS));
    let report = Report {
        schema_version: 1,
        fixture_version: FIXTURE_VERSION,
        build_label,
        rustc: String::from_utf8(
            std::process::Command::new("rustc")
                .arg("--version")
                .output()
                .expect("rustc version")
                .stdout,
        )
        .expect("UTF-8 rustc version")
        .trim()
        .into(),
        platform: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        build_environment: [
            "RUSTFLAGS",
            "CFLAGS",
            "TARGET_CPU",
            "CARGO_PROFILE_TEST_OPT_LEVEL",
            "CARGO_PROFILE_PHOTO_PERF_OPT_LEVEL",
        ]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
        .collect(),
        logical_cpus: std::thread::available_parallelism()
            .expect("CPU count")
            .get(),
        variant_concurrency: config.photos.variant_concurrency,
        webp_quality: config.photos.webp_quality,
        variant_max_edges: VariantKind::ALL.map(|k| k.max_edge()).to_vec(),
        warmup_batches: 1,
        measured_batches: ITERATIONS,
        fixtures: inputs,
        batch_wall_ms,
        samples,
        summary,
    };
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).expect("report directory");
    }
    fs::write(
        &output,
        serde_json::to_vec_pretty(&report).expect("report JSON"),
    )
    .expect("write report");
    println!("\nfixture/variant/phase | median ms | min ms | max ms | baseline ms | speedup");
    for (key, metric) in &report.summary {
        let comparison = baseline
            .as_ref()
            .and_then(|b| b.summary.get(key))
            .map_or_else(
                || "- | -".into(),
                |b| {
                    format!(
                        "{:.3} | {:.2}x",
                        b.median_ms,
                        b.median_ms / metric.median_ms
                    )
                },
            );
        println!(
            "{key} | {:.3} | {:.3} | {:.3} | {comparison}",
            metric.median_ms, metric.min_ms, metric.max_ms,
        );
    }
    println!("Report: {}", output.display());
    if let Some(baseline) = baseline {
        performance::compare(&report.summary, &baseline.summary, tolerance)
            .expect("phase performance regression");
    }
    drop(queue);
    provider.shutdown().expect("shutdown exporter");
}
