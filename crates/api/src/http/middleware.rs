//! Request-scoped middleware: request id generation and propagation, tracing
//! spans, and timeouts.
//!
//! The request id must reach the client (header at minimum) and appear on
//! every log line for that request. That pairing is what makes the design's
//! "Reference 7f3a-2b91" error screen actually useful — the user quotes it and
//! it finds the log.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode};
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::OnResponse;
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::config::ServerConfig;

/// Global request body limit: 32 MiB, including multipart framing. Uploads
/// also enforce their configured request limit; buffered JSON extractors use
/// axum's smaller default limit.
const MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Applies the middleware stack. Order matters: request id is outermost so
/// that even a rejection from an inner layer carries one.
///
/// Takes the server config because two of the layers are configured from it.
/// [`crate::http::router`] is the only caller, and it has the state to hand.
pub fn apply(router: Router, config: &ServerConfig) -> Router {
    // ServiceBuilder lists layers from outermost to innermost: set request ID,
    // propagate it to the response, trace, time out, then limit the body.
    // Client-supplied request IDs are preserved; they are not trusted identifiers.
    let stack = ServiceBuilder::new()
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuidV7))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_request_span)
                // INFO, not the DEBUG default: this is the access log, and it
                // is the line that carries the request id, latency and status.
                .on_response(
                    |response: &axum::http::Response<tower_http::limit::ResponseBody<Body>>,
                     latency: Duration,
                     span: &tracing::Span| {
                        DefaultOnResponse::new()
                            .level(Level::INFO)
                            .on_response(response, latency, span);
                        span.record("http.response.status_code", response.status().as_u16());
                        if response.status().is_server_error() {
                            span.record("otel.status_code", "ERROR");
                        }
                    },
                )
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .map_response(|mut response: axum::http::Response<_>| {
            crate::telemetry::inject_context(
                &tracing::Span::current().context(),
                response.headers_mut(),
            );
            response
        })
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(config.request_timeout_secs),
        ))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES))
        // `RequestBodyLimit` hands its inner service a `Limited<Body>`, but
        // axum's router only speaks `axum::body::Body`. Mapping it back is
        // what makes the two compose; the limit is still enforced, because
        // `Body::new` re-wraps the limited body rather than unwrapping it, so
        // an oversized chunked body still fails mid-stream — not only the
        // ones that declare an honest `Content-Length`.
        .map_request(|req: Request<_>| req.map(Body::new));

    router.layer(stack)
}

/// Builds the per-request span.
///
/// The span is deliberately `INFO`. A `DEBUG` span would be filtered out under
/// the default `info` directive, and a span that is not recorded contributes
/// no fields — the request id would silently vanish from every log line, which
/// is the one thing this middleware exists to guarantee.
fn make_request_span(request: &Request<Body>) -> tracing::Span {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .unwrap_or("unknown");

    // Fields are flat and low-cardinality-first so JSON output stays greppable.
    // Exclude query strings: authentication callbacks carry OAuth `code` and
    // `state` values that must not appear in logs.
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or("unknown", |path| path.as_str());
    let span = tracing::info_span!(parent: None,
        "http_request",
        otel.name = %format!("{} {}", request.method(), route),
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
        http.request.method = %request.method(),
        http.route = %route,
        http.response.status_code = tracing::field::Empty,
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path(),
        version = ?request.version(),
    );
    let _ = span.set_parent(crate::telemetry::extract_context(request.headers()));
    span
}

/// A [`MakeRequestId`] producing UUIDv7s.
///
/// v7 rather than tower-http's built-in v4 `MakeRequestUuid`: the first 48 bits
/// are a millisecond timestamp, so ids sort chronologically. When a user quotes
/// a reference from an error screen, an ordered id narrows the log search to a
/// point in time by itself.
#[derive(Clone, Copy, Default)]
struct MakeRequestUuidV7;

impl MakeRequestId for MakeRequestUuidV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        // A hyphenated UUID is always visible ASCII, so the parse cannot fail;
        // `Option` exists for generators that can, and returning `None` here
        // would simply mean "no id", which is not worth a panic.
        HeaderValue::from_str(&Uuid::now_v7().to_string())
            .ok()
            .map(RequestId::new)
    }
}
