//! Tracing setup.
//!
//! Console/JSON logs and optional OpenTelemetry export. Request context also
//! follows per-photo jobs and CPU work; see docs/tracing.md.

use axum::http::{HeaderMap, HeaderValue};
use opentelemetry::{
    Context,
    propagation::{Extractor, Injector, TextMapPropagator},
    trace::TracerProvider as _,
};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use tracing::Subscriber;
use tracing_subscriber::{
    EnvFilter, Layer, fmt, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt,
};

use crate::config::TelemetryConfig;

#[derive(Debug, thiserror::Error)]
#[error("could not initialize telemetry")]
pub struct TelemetryError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

/// Installs the global subscriber. Called once, early, before anything that
/// might want to log a failure.
///
/// Under the `tokio-console` feature this also installs the console layer —
/// see D8. That layer is how a blocking task holding a worker thread becomes
/// visible, and it must never be present in a release build.
pub fn init(config: &TelemetryConfig) -> Result<Option<SdkTracerProvider>, TelemetryError> {
    // `RUST_LOG` wins over the file so a single shell can be made verbose
    // without editing anything. An invalid `RUST_LOG` is a hard failure rather
    // than a silent fallback: quietly ignoring it means the operator spends
    // the next ten minutes wondering where their logs went.
    let directives = std::env::var(EnvFilter::DEFAULT_ENV).ok();
    init_with_directives(config, directives.as_deref())
}

fn init_with_directives(
    config: &TelemetryConfig,
    environment_directives: Option<&str>,
) -> Result<Option<SdkTracerProvider>, TelemetryError> {
    let directives = environment_directives.unwrap_or(&config.log_level);
    let filter = EnvFilter::try_new(directives).map_err(|err| TelemetryError(Box::new(err)))?;

    let provider = build_provider(config)?;
    let otel = provider.as_ref().map(|provider| {
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer("pinvoyager-api"))
            .with_filter(SpanOnly)
    });
    let registry = tracing_subscriber::registry().with(otel);

    // D8. The console layer stays unfiltered and the `EnvFilter` is attached
    // to the fmt layer alone — the runtime instrumentation the console needs
    // is emitted at TRACE on `tokio`/`runtime` targets, so a subscriber-wide
    // filter would discard exactly the events that make it work.
    #[cfg(feature = "tokio-console")]
    let registry = registry.with(console_subscriber::spawn());

    registry
        .with(fmt_layer(config.json).with_filter(filter))
        // `try_init` rather than `init`: a second call — a test harness, an
        // embedder — should get an error it can inspect, not a panic.
        .try_init()
        .map_err(|err| TelemetryError(Box::new(err)))?;
    Ok(provider)
}

/// The formatting layer, boxed because the JSON and pretty formatters are
/// different types and this is the one place that difference matters.
fn fmt_layer<S>(json: bool) -> Box<dyn Layer<S> + Send + Sync>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    if json {
        Box::new(
            fmt::layer()
                .json()
                // Event fields at the top level, and the enclosing spans kept
                // alongside them: the per-request span carries the request id,
                // and a log line that cannot be tied back to a request is
                // most of the way to useless.
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true),
        )
    } else {
        Box::new(fmt::layer().pretty())
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;

/// Bounded background exports; no request awaits the collector.
fn build_provider(config: &TelemetryConfig) -> Result<Option<SdkTracerProvider>, TelemetryError> {
    let Some(endpoint) = config
        .otlp_endpoint
        .as_ref()
        .filter(|endpoint| !endpoint.is_empty())
    else {
        return Ok(None);
    };
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(endpoint)
        .with_timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|err| TelemetryError(Box::new(err)))?;
    Ok(Some(
        SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
            .with_resource(
                Resource::builder()
                    .with_service_name(config.service_name.clone())
                    .build(),
            )
            .build(),
    ))
}

struct Headers<'a>(&'a HeaderMap);
impl Extractor for Headers<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}
struct ResponseHeaders<'a>(&'a mut HeaderMap);
impl Injector for ResponseHeaders<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            key.parse::<axum::http::HeaderName>(),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}
pub fn extract_context(headers: &HeaderMap) -> Context {
    TraceContextPropagator::new().extract(&Headers(headers))
}
pub fn inject_context(context: &Context, headers: &mut HeaderMap) {
    TraceContextPropagator::new().inject_context(context, &mut ResponseHeaders(headers));
}

/// Flush after HTTP draining, off the async runtime worker threads.
pub async fn shutdown(provider: Option<SdkTracerProvider>) {
    if let Some(provider) = provider {
        let result = tokio::task::spawn_blocking(move || provider.shutdown()).await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!("trace exporter shutdown did not complete successfully");
        }
    }
}

/// Keep callsites enabled so sibling log filters cannot erase span context.
/// Discard log events at the event callback rather than filtering their metadata.
#[derive(Clone, Copy)]
pub struct SpanOnly;
impl<S: Subscriber> tracing_subscriber::layer::Filter<S> for SpanOnly {
    fn enabled(
        &self,
        _metadata: &tracing::Metadata<'_>,
        _context: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        true
    }
    fn event_enabled(
        &self,
        _event: &tracing::Event<'_>,
        _context: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        false
    }
}
