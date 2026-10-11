# PinVoyager

PinVoyager is a photo travelogue application. Owners create a voyage, choose its
public location precision, and upload photos. The API reads EXIF locations,
processes image variants, and supplies the voyage's map and photo views.

Draft voyages are owner-only. Published voyages and named public profiles are
readable without an account. Authentication uses Google, Microsoft, Apple, or
Facebook identities and server-side sessions.

## Documentation

- [Architecture](docs/architecture.md): system boundaries, data model, API, and processing lifecycle.
- [Documentation inventory](docs/documentation-inventory.md): current functional and operational references.
- [Running locally](docs/running.md): dependencies, configuration, and startup.
- [Testing](docs/testing.md): test suites, coverage, and verification commands.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/api/` | Rust API using axum, Tokio, sqlx, and PostgreSQL; migrations and tests. |
| `web/` | React/TypeScript application using Vite, React Query, React Router, and MapLibre. |
| `design/tokens.css` | Shared color, typography, spacing, and motion tokens imported by the app. |
| `design/components.css` | Shared component styles imported by the app. |
| `docs/` | Architecture, implemented behavior, and operating instructions. |
| `.sqlx/` | Offline metadata for checked SQL queries. |
| `dev/` | Development OpenTelemetry Collector configuration. |

The browser uses OpenStreetMap raster tiles in development. Image originals stay
in local storage; authorized image endpoints serve metadata-stripped WebP
variants. Background processing runs inside the API process.

Development Compose runs PostgreSQL, an OpenTelemetry Collector, and Jaeger.
See [tracing](docs/tracing.md) for inspecting requests and image jobs and
[photo performance](docs/photo-performance.md) for the opt-in worker benchmark.
