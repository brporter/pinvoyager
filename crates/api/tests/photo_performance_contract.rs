//! Fast checks for the on-demand benchmark's measurement and regression logic.
#[path = "support/photo_performance.rs"]
mod performance;

use std::time::{Duration, SystemTime};

use opentelemetry::{
    KeyValue,
    trace::{SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState},
};
use opentelemetry_sdk::trace::SpanData;
use performance::{Sample, compare, photo_samples, summarize};

fn span(id: u64, parent: u64, name: String, attributes: Vec<KeyValue>) -> SpanData {
    SpanData {
        span_context: SpanContext::new(
            TraceId::from(1),
            SpanId::from(id),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::from(parent),
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: name.into(),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH + Duration::from_millis(10),
        attributes,
        dropped_attributes_count: 0,
        events: Default::default(),
        links: Default::default(),
        status: Status::Ok,
        instrumentation_scope: Default::default(),
    }
}

fn exported_photo() -> Vec<SpanData> {
    let mut spans = vec![
        span(
            1,
            0,
            "photo.process id-1".into(),
            vec![KeyValue::new("photo.id", "id-1")],
        ),
        span(2, 1, "photo.generate".into(), vec![]),
    ];
    for (i, kind) in ["thumb", "display", "full"].into_iter().enumerate() {
        let id = 3 + i as u64 * 3;
        spans.push(span(
            id,
            2,
            "photo.variant".into(),
            vec![KeyValue::new("photo.variant", kind)],
        ));
        spans.push(span(id + 1, id, "photo.resize".into(), vec![]));
        spans.push(span(id + 2, id, "photo.encode".into(), vec![]));
    }
    spans
}

#[test]
fn measurements_follow_each_photo_and_variant_parent_in_exported_spans() {
    let mut spans = exported_photo();
    spans.push(span(50, 99, "photo.resize".into(), vec![]));
    let mut other_trace = span(51, 3, "photo.resize".into(), vec![]);
    other_trace.span_context = SpanContext::new(
        TraceId::from(2),
        SpanId::from(51),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );
    spans.push(other_trace);
    let samples = photo_samples(&spans, "id-1", "jpeg-12mp", 1).expect("six phase measurements");
    assert_eq!(samples.len(), 6);
    assert!(samples.iter().all(|s| s.milliseconds == 10.0
        && s.photo_id == "id-1"
        && s.fixture == "jpeg-12mp"
        && s.iteration == 1));
    assert_eq!(
        samples.iter().filter(|s| s.phase == "photo.resize").count(),
        3
    );
}

#[test]
fn missing_duplicate_failed_or_nonpositive_spans_cannot_look_like_a_speedup() {
    let spans = exported_photo();
    assert!(photo_samples(&[], "id-1", "fixture", 1).is_err());
    for index in [0, 1, 2, 3, 4] {
        let mut missing = spans.clone();
        missing.remove(index);
        assert!(photo_samples(&missing, "id-1", "fixture", 1).is_err());
        let mut duplicate = spans.clone();
        duplicate.push(spans[index].clone());
        assert!(photo_samples(&duplicate, "id-1", "fixture", 1).is_err());
    }
    let mut failed = spans.clone();
    failed[0].status = Status::error("failed");
    assert!(photo_samples(&failed, "id-1", "fixture", 1).is_err());
    let mut wrong_variant = spans.clone();
    wrong_variant[2].attributes = vec![KeyValue::new("photo.variant", "unknown")];
    assert!(photo_samples(&wrong_variant, "id-1", "fixture", 1).is_err());
    for end in [
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH - Duration::from_secs(1),
    ] {
        let mut invalid = spans.clone();
        invalid[3].end_time = end;
        assert!(photo_samples(&invalid, "id-1", "fixture", 1).is_err());
    }
}

fn samples(values: &[f64]) -> Vec<Sample> {
    values
        .iter()
        .enumerate()
        .map(|(i, ms)| Sample {
            fixture: "jpeg-12mp".into(),
            iteration: i + 1,
            photo_id: format!("id-{i}"),
            trace_id: "trace".into(),
            variant: "thumb".into(),
            phase: "photo.resize".into(),
            milliseconds: *ms,
        })
        .collect()
}

#[test]
fn summary_uses_median_and_preserves_range_and_sample_count() {
    let report = summarize(&samples(&[30.0, 10.0, 20.0])).expect("summary");
    let metric = &report["jpeg-12mp/thumb/photo.resize"];
    assert_eq!(
        (
            metric.samples,
            metric.median_ms,
            metric.min_ms,
            metric.max_ms
        ),
        (3, 20.0, 10.0, 30.0)
    );
    assert_eq!(
        summarize(&samples(&[10.0, 20.0]))
            .expect("even median")
            .values()
            .next()
            .expect("metric")
            .median_ms,
        15.0
    );
    for invalid in [
        vec![],
        samples(&[0.0]),
        samples(&[f64::NAN]),
        samples(&[f64::INFINITY]),
        samples(&[-1.0]),
    ] {
        assert!(summarize(&invalid).is_err());
    }
    let mut duplicate = samples(&[10.0]);
    duplicate.push(duplicate[0].clone());
    assert!(summarize(&duplicate).is_err());
}

#[test]
fn comparison_rejects_regressions_and_missing_metrics_but_allows_noise() {
    let baseline = summarize(&samples(&[100.0])).expect("baseline");
    for acceptable in [50.0, 121.9, 122.0] {
        compare(
            &summarize(&samples(&[acceptable])).expect("current"),
            &baseline,
            20.0,
        )
        .expect("within 20% plus 2 ms");
    }
    let error = compare(
        &summarize(&samples(&[123.0])).expect("current"),
        &baseline,
        20.0,
    )
    .expect_err("regression");
    assert!(error.contains("jpeg-12mp/thumb/photo.resize"));
    assert!(compare(&Default::default(), &baseline, 20.0).is_err());
    assert!(compare(&baseline, &Default::default(), 20.0).is_err());
    let mut different_key = samples(&[100.0]);
    different_key[0].fixture = "different-fixture".into();
    assert!(
        compare(
            &summarize(&different_key).expect("summary"),
            &baseline,
            20.0
        )
        .is_err()
    );
    for invalid_ms in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut invalid = baseline.clone();
        invalid.values_mut().next().expect("metric").median_ms = invalid_ms;
        assert!(compare(&baseline, &invalid, 20.0).is_err());
        assert!(compare(&invalid, &baseline, 20.0).is_err());
    }
    let mut empty_count = baseline.clone();
    empty_count.values_mut().next().expect("metric").samples = 0;
    assert!(compare(&empty_count, &baseline, 20.0).is_err());
    for tolerance in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(compare(&baseline, &baseline, tolerance).is_err());
    }
}
