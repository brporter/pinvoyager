//! Tracing setup.
//!
//! CONTRACT — `init` signature is fixed; implementation owned by the config /
//! telemetry agent. JSON output in production, per-request spans, and the
//! request id correlated into every line emitted during a request.

use tracing::Subscriber;
use tracing_subscriber::{
    EnvFilter, Layer, fmt, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt,
};

use crate::config::TelemetryConfig;

#[derive(Debug, thiserror::Error)]
#[error("could not initialise telemetry")]
pub struct TelemetryError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

/// Installs the global subscriber. Called once, early, before anything that
/// might want to log a failure.
///
/// Under the `tokio-console` feature this also installs the console layer —
/// see D8. That layer is how a blocking task holding a worker thread becomes
/// visible, and it must never be present in a release build.
pub fn init(config: &TelemetryConfig) -> Result<(), TelemetryError> {
    // `RUST_LOG` wins over the file so a single shell can be made verbose
    // without editing anything. An invalid `RUST_LOG` is a hard failure rather
    // than a silent fallback: quietly ignoring it means the operator spends
    // the next ten minutes wondering where their logs went.
    let directives =
        std::env::var(EnvFilter::DEFAULT_ENV).unwrap_or_else(|_| config.log_level.clone());
    let filter = EnvFilter::try_new(&directives).map_err(|err| TelemetryError(Box::new(err)))?;

    let registry = tracing_subscriber::registry();

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
        .map_err(|err| TelemetryError(Box::new(err)))
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
