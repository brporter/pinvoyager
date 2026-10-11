# Tracing voyage creation and image processing

For repeatable large-image timing measurements and saved-baseline checks, see the
[on-demand photo performance regression](photo-performance.md). It measures the resize
and encode spans in unoptimized and optimized builds.

The development API exports OpenTelemetry traces over OTLP/HTTP to an
OpenTelemetry Collector, which forwards them to Jaeger. Jaeger's timeline shows
voyage creation, each upload, time spent waiting for worker capacity, and the
individual photo conversion stages. The [image pipeline](photo-pipeline.md) describes storage, queueing, processing, and readiness behavior.

## Start and review a flow

From the repository root:

```sh
docker compose up -d
cargo run -p pinvoyager-api
```

In a second terminal:

```sh
npm --prefix web run dev
```

Open the app at `http://localhost:5173`, sign in, start a new voyage, and upload
several photos through the creation screen. Continue to review and the voyage
page as usual. Open [Jaeger](http://localhost:16686), choose service
**pinvoyager-api**, choose operation **POST /api/voyages**, and search recent
traces. Open the matching trace and expand the upload branches. Refresh the
trace after processing finishes if some spans have not arrived yet.

Each photo has a distinct operation named `photo.process <photo UUID>` and a
`photo.id` attribute. Live uploads also annotate it with `voyage.id`, source
format, source dimensions, and final processing status. Expand each one to see
`photo.generate`, `photo.decode`, orientation, and the three `photo.variant`
branches (`thumb`, `display`, `full`). Each variant has separate resize, encode,
and write spans. Filenames and photo content are deliberately absent from tags;
the photo UUID matches the upload response and voyage JSON.

Use Jaeger's Tags search to find a voyage or photo, for example:

```json
{"voyage.id":"<voyage UUID>"}
```

For a direct trace link, inspect the creation response's `traceparent` in the
browser's Network panel. The middle 32-character field is the trace ID:

```text
00-<trace ID>-<span ID>-01
http://localhost:16686/trace/<trace ID>
```

A trace can contain uploads and conversions that finish after its creation HTTP span
ends. The creation request provides the parent context for later requests and ends
before file selection and background processing finish. Gaps between creation and
uploads can include user interaction time and should not be interpreted as server
computation.

## Trace shape and propagation

```mermaid
flowchart LR
    Browser["Upload screen"] -->|"POST create; receive traceparent"| Create["HTTP creation span / voyage.create"]
    Browser -->|"POST each file with creation traceparent"| Upload["HTTP upload span / photo.upload / photo.ingest"]
    Create -.->|"same trace, creation HTTP span as parent"| Upload
    Upload --> Wait["photo.queue_wait: one per photo"]
    Upload --> Process["photo.process UUID: one per photo"]
    Process --> Generate["photo.generate"]
    Generate --> Decode["photo.decode and photo.orient"]
    Generate --> Variant["photo.variant: thumb, display, full"]
    Variant --> Resize["photo.resize"]
    Variant --> Encode["photo.encode"]
    Variant --> Write["photo.write"]
    Generate --> Ready["photo.mark_ready, or photo.mark_failed"]
```

The HTTP middleware extracts W3C `traceparent` and `tracestate` headers and makes
the request span a child of that context. Missing or malformed context starts a
new root; it does not reject a request. Responses include trace context when
tracing is enabled. HTTP spans record method, route template, request ID, path
without query string, status, and server-error status.

`RequestOptions.traceContext` is an optional mutable object. `createVoyage` captures the
response's `traceparent` into it, and `Upload` holds the object in a React ref for the
life of that screen. Each XHR upload sends the same creation parent, so uploads appear
as sibling branches under the creation request. The browser only propagates context; it
does not install a browser tracing SDK or export directly to the Collector.

`VariantJob::new` captures the current OpenTelemetry context and starts a separate
`photo.queue_wait` span. The job carries this context in the in-memory queue. The
dispatcher ends queue wait on receive, then processing starts a separately named span
with the captured upload context. Keeping OpenTelemetry context instead of a cloned
request `tracing::Span` lets the HTTP request end normally while processing continues.

Both blocking-work helpers capture the current span and tracing dispatcher,
then enter them inside `spawn_blocking`. Nested decode/resize/encode/write spans
therefore stay under the correct photo even when jobs run concurrently on
separate threads. No database connection is held across conversion.

There is no database trace column and no persistent browser trace context.
A page reload or a separately opened existing-voyage upload starts a new request
trace. Startup recovery has its own `photo.recover` trace; recovered jobs keep
`photo.id`, but cannot recover the original upload trace or the optional
in-memory voyage tag. Publishing and readiness reads after navigation normally
start separate HTTP traces; they do not delay or participate in conversion.

## What the timings mean

| Span | What to inspect |
| --- | --- |
| `voyage.create` | Draft validation and database insertion |
| `photo.upload` | Ownership check and complete upload handling |
| `photo.ingest` | One file's streamed transfer, validation, storage, insertion, and enqueue |
| `photo.validate` | Read dimensions and check the maximum pixel count |
| `photo.hash` | SHA-256 over the stored temporary original |
| `photo.exif` | Metadata read, during ingest or before conversion |
| `photo.store` | Original storage commit/deduplication |
| `photo.enqueue` | Time waiting for a free slot in the worker queue |
| `photo.queue_wait` | From job construction after acceptance to dispatcher receipt |
| `photo.process <UUID>` | Actual processing task, excluding queue wait |
| `photo.load` | Read the photo row before conversion |
| `photo.decode` | Decode original pixels; `photo.format` identifies the decoder path |
| `photo.orient` | Apply orientation to decoded pixels |
| `photo.variant` | One output size, tagged with variant name and maximum edge |
| `photo.resize` / `photo.encode` / `photo.write` | Resampling, lossy WebP encoding, atomic variant write |
| `photo.mark_ready` / `photo.mark_failed` | Save the worker result |

For uploads, the handler reserves a queue slot before constructing the job.
`photo.enqueue` measures time waiting for that slot; `photo.queue_wait` measures time
from job construction to dispatcher receipt. Photo processing spans can overlap because
the worker allows four active jobs by default. The worker still marks a photo ready only
after all three variants have been written. A thumbnail written earlier remains
unavailable until that update succeeds. The voyage still polls once per second while any
photo is pending. Browser image fetching, lazy loading, decoding, and painting are not
measured by this server trace; use the browser Network/Performance panels for that
interval.

When processing fails, the worker sets the span's OpenTelemetry error status and
attempts to mark the photo row `failed`. Application logs record the cause; log events
are not exported. If the status write fails, the row can remain pending for a later
recovery pass. Workers skip conversion for deleted or already-ready photos. Recovery can
queue a photo while a live upload is also queueing it, so one photo can have two
processing spans. See [conversion and
recovery](photo-pipeline.md#conversion-and-recovery).

## Development services and configuration

```mermaid
flowchart LR
    API["Rust API: tracing + OpenTelemetry SDK"] -->|"OTLP HTTP/protobuf /v1/traces"| Collector["Collector :4318"]
    Collector -->|"OTLP HTTP; internal Compose network"| Jaeger["Jaeger all-in-one; in-memory traces"]
    User["Developer browser"] -->|"localhost:16686"| Jaeger
```

[`docker-compose.yml`](../docker-compose.yml) starts PostgreSQL, Collector, and Jaeger
without a profile. `--profile api` also starts the API container. The Collector accepts
OTLP/HTTP, limits its memory, batches spans, and forwards them to Jaeger over the
Compose network. Only Collector HTTP port 4318 and Jaeger UI port 16686 are exposed,
both on loopback. Jaeger's receiver ports remain internal to Compose.

| Setting | Value / behavior |
| --- | --- |
| `PINVOYAGER__TELEMETRY__OTLP_ENDPOINT` | Full traces URL: host API `http://127.0.0.1:4318/v1/traces`; container API `http://otel-collector:4318/v1/traces` |
| `PINVOYAGER__TELEMETRY__SERVICE_NAME` | `pinvoyager-api`, the Jaeger service selector |
| `OTEL_HTTP_PORT` | Compose host port, default 4318; update host API endpoint too if changed |
| `JAEGER_PORT` | Compose UI port, default 16686 |
| `RUST_LOG` / `PINVOYAGER__TELEMETRY__LOG_LEVEL` | Log verbosity; diagnostic spans are exported independently |

Built-in configuration disables export when the endpoint is absent. Committed
`config.toml` enables the local endpoint. An empty endpoint also disables export:

```sh
PINVOYAGER__TELEMETRY__OTLP_ENDPOINT='' cargo run -p pinvoyager-api
```

OpenTelemetry sampling records every local root trace and honors incoming W3C sampling
decisions. An incoming trace whose flag is `00` is not exported. SDK exports run in
background batches using the SDK's queue capacity and a three-second exporter timeout.
Uploads never await delivery to the Collector. If the Collector is down or the
Collector/exporter queues fill, telemetry can be lost without changing the upload
outcome. On normal exit, `main::run_service` waits for HTTP requests and then image
jobs, subject to their separate shutdown timeouts. `telemetry::shutdown` then flushes
completed spans on the blocking pool. Abrupt termination can lose spans. See [shutdown
order](architecture.md#startup-request-handling-and-shutdown).

Jaeger uses transient in-memory storage: traces disappear when it restarts and
are not an audit trail. No Redis, Elasticsearch, Grafana, or persistent trace
database is needed for this development setup. See the official
[Jaeger all-in-one documentation](https://www.jaegertracing.io/docs/2.21/getting-started/)
and [Collector configuration](https://opentelemetry.io/docs/collector/configuration/).

## Implementation and checks

| Source | Symbols / responsibility |
| --- | --- |
| [`telemetry.rs`](../crates/api/src/telemetry.rs) | `init`, `build_provider`, `extract_context`, `inject_context`, `shutdown`: export, propagation, log coexistence, flush |
| [`config.rs`](../crates/api/src/config.rs) | `TelemetryConfig`: endpoint and service name |
| [`middleware.rs`](../crates/api/src/http/middleware.rs) | `make_request_span`, `apply`: incoming parent and outgoing response context |
| [`voyages.rs`](../crates/api/src/http/voyages.rs) | `create`: voyage ID and creation span |
| [`photos.rs`](../crates/api/src/http/photos.rs) | `upload_to_voyage`, `ingest_part`: upload and per-file stages |
| [`worker.rs`](../crates/api/src/photos/worker.rs) | `VariantJob`, `dispatch`, `process`, `generate`, `encode`, `write_variants`: queue context and separate photo/variant spans |
| [`blocking.rs`](../crates/api/src/blocking.rs) | `run_blocking`, `run_blocking_infallible`: thread context transfer |
| [`variants.rs`](../crates/api/src/photos/variants.rs) | `encode_variant`: separate resizing and encoding |
| [`client.ts`](../web/src/api/client.ts), [`Upload.tsx`](../web/src/screens/Upload.tsx) | `TraceContext`, `request`, `uploadPhotos`, `Upload`: creation-to-upload header propagation |
| [`otel-collector.yaml`](../dev/otel-collector.yaml) | Receivers, processors, exporter, trace pipeline |

[`image_tracing.rs`](../crates/api/tests/image_tracing.rs) tests exported spans through
the application router, PostgreSQL, worker queue, background tasks, and blocking
conversion. It checks one shared trace, two individually identified photos, six variant
spans, stage names, missing/malformed and unsampled parents, failed processing, and
absence of uploaded filenames. The subscriber includes the log layer as well as the
tracing layer. [`worker_tests.rs`](../crates/api/src/photos/worker_tests.rs) checks that
queued context does not keep the upload span open.
[`telemetry_tests.rs`](../crates/api/src/telemetry_tests.rs) covers optional export,
invalid configuration, W3C extraction/injection, and shutdown.

Frontend coverage includes real client header propagation in
[`clientUpload.test.ts`](../web/src/api/__tests__/clientUpload.test.ts) and the
multi-file screen flow in
[`VoyageUploadFlow.test.tsx`](../web/src/screens/__tests__/VoyageUploadFlow.test.tsx).


```sh
npm --prefix web run test -- --coverage
npm --prefix web run typecheck
npm --prefix web run lint
npm --prefix web run build

DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager' SQLX_OFFLINE=true cargo test --test image_tracing
DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager' SQLX_OFFLINE=true cargo coverage -- --test-threads=1
cargo fmt --check
SQLX_OFFLINE=true cargo clippy --all-targets -- -D warnings
```
