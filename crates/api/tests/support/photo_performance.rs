use std::collections::{BTreeMap, BTreeSet};

use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::SpanData;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub fixture: String,
    pub iteration: usize,
    pub photo_id: String,
    pub trace_id: String,
    pub variant: String,
    pub phase: String,
    pub milliseconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub samples: usize,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

pub fn photo_samples(
    spans: &[SpanData],
    photo_id: &str,
    fixture: &str,
    iteration: usize,
) -> Result<Vec<Sample>, String> {
    fn one<'a>(matches: Vec<&'a SpanData>, description: &str) -> Result<&'a SpanData, String> {
        if matches.len() != 1 {
            Err(format!(
                "expected one {description}, found {}",
                matches.len()
            ))
        } else {
            Ok(matches[0])
        }
    }
    let processing = one(
        spans
            .iter()
            .filter(|s| s.name == format!("photo.process {photo_id}"))
            .collect(),
        "photo.process",
    )?;
    if matches!(processing.status, Status::Error { .. }) {
        return Err(format!("{photo_id}: processing failed"));
    }
    let children = |parent: &SpanData, name: &str| {
        spans
            .iter()
            .filter(|s| {
                s.parent_span_id == parent.span_context.span_id()
                    && s.span_context.trace_id() == parent.span_context.trace_id()
                    && s.name == name
            })
            .collect::<Vec<_>>()
    };
    let generate = one(children(processing, "photo.generate"), "photo.generate")?;
    let variants = children(generate, "photo.variant");
    if variants.len() != 3 {
        return Err(format!("{photo_id}: expected three variants"));
    }
    let mut samples = Vec::new();
    for kind in ["thumb", "display", "full"] {
        let variant = one(
            variants
                .iter()
                .copied()
                .filter(|s| {
                    s.attributes
                        .iter()
                        .any(|a| a.key.as_str() == "photo.variant" && a.value.to_string() == kind)
                })
                .collect(),
            kind,
        )?;
        for phase in ["photo.resize", "photo.encode"] {
            let span = one(children(variant, phase), phase)?;
            let elapsed = span
                .end_time
                .duration_since(span.start_time)
                .map_err(|_| format!("{photo_id}: {phase} ends before it starts"))?;
            if elapsed.is_zero() {
                return Err(format!("{photo_id}: {phase} has zero duration"));
            }
            samples.push(Sample {
                fixture: fixture.into(),
                iteration,
                photo_id: photo_id.into(),
                trace_id: span.span_context.trace_id().to_string(),
                variant: kind.into(),
                phase: phase.into(),
                milliseconds: elapsed.as_secs_f64() * 1000.0,
            });
        }
    }
    Ok(samples)
}

pub fn summarize(samples: &[Sample]) -> Result<BTreeMap<String, Summary>, String> {
    if samples.is_empty() {
        return Err("no performance samples".into());
    }
    let mut groups: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for sample in samples {
        let key = format!("{}/{}/{}", sample.fixture, sample.variant, sample.phase);
        if !sample.milliseconds.is_finite() || sample.milliseconds <= 0.0 {
            return Err(format!("{key}: duration must be positive and finite"));
        }
        if !seen.insert((key.clone(), sample.iteration)) {
            return Err(format!("{key}: duplicate iteration"));
        }
        groups.entry(key).or_default().push(sample.milliseconds);
    }
    Ok(groups
        .into_iter()
        .map(|(key, mut values)| {
            values.sort_by(f64::total_cmp);
            let n = values.len();
            (
                key,
                Summary {
                    samples: n,
                    median_ms: (values[(n - 1) / 2] + values[n / 2]) / 2.0,
                    min_ms: values[0],
                    max_ms: values[n - 1],
                },
            )
        })
        .collect())
}

pub fn compare(
    current: &BTreeMap<String, Summary>,
    baseline: &BTreeMap<String, Summary>,
    max_regression_percent: f64,
) -> Result<(), String> {
    if !max_regression_percent.is_finite() || max_regression_percent < 0.0 {
        return Err("regression tolerance must be finite and nonnegative".into());
    }
    if current.is_empty() || !current.keys().eq(baseline.keys()) {
        return Err("baseline metric set differs from this run".into());
    }
    let mut failures = Vec::new();
    for (key, value) in current {
        let reference = &baseline[key];
        if [value, reference]
            .iter()
            .any(|s| s.samples == 0 || !s.median_ms.is_finite() || s.median_ms <= 0.0)
        {
            return Err(format!("{key}: invalid baseline or current summary"));
        }
        // A small absolute allowance prevents tiny spans being judged on timer noise.
        let limit = reference.median_ms * (1.0 + max_regression_percent / 100.0) + 2.0;
        if value.median_ms > limit {
            failures.push(format!(
                "{key}: {:.3} ms exceeds {:.3} ms (baseline {:.3} ms)",
                value.median_ms, limit, reference.median_ms
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
