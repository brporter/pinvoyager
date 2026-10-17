# Documentation inventory

The documentation covers current architecture, implemented behavior, and operation.
Start with [architecture](architecture.md) and [running locally](running.md).

## Architecture

| Document | Purpose |
| --- | --- |
| [Architecture overview](architecture.md) | System boundaries, modules, data model, API routes, authentication, uploads, maps, frontend state, and operational limits. |
| [Implementation constraints](architecture/constraints.md) | Current privacy, identity, storage, processing, map, and naming boundaries; explains D identifiers in source comments. |
| [Navigation](architecture/navigation.md) | Routes, layouts, sign-in/name prompts, and navigation between implemented workflows. |
| [Authentication flow](architecture/auth-flow.md) | Supported providers, provider exchange and retirement, browser binding, account linking, session cookies, expiry, and logout. |
| [Application data flow](architecture/data-flow.md) | Client transport, query/cache behavior, handlers, SQL, and response construction. |
| [Map data flow](architecture/map-data-flow.md) | Voyage/discovery inputs, clustering, coordinate precision, selection, and antimeridian/precision behavior. |
| [Image pipeline](photo-pipeline.md) | Upload validation, storage, background queue, conversion, readiness polling, recovery, and authorized serving. |

## Functional behavior

| Document | Purpose |
| --- | --- |
| [Voyage creation and photo ownership](voyage-owned-photos.md) | Creation, default and selected precision, upload/review, publication, covers, deletion, and ownership migration. |
| [Location gallery](location-gallery.md) | Marker membership, photo navigation, processing states, native dialog dismissal, and focus handling. |
| [Voyage sharing](voyage-sharing.md) | Public URL copying, draft publication, errors, and dialog lifecycle. |
| [Display names and profiles](display-names.md) | Name validation, unique claims, reservations, aliases, migration, and anonymous published-voyage profiles. |

## Operation and verification

| Document | Purpose |
| --- | --- |
| [Production deployment](deployment.md) | GHCR publication, VM containers, Caddy routing, credentials, verification, persistence, and rollback. |
| [Running locally](running.md) | Native dependencies, PostgreSQL, configuration, startup, development login, SQL metadata, and persistent container storage. |
| [Testing](testing.md) | Backend/frontend commands, coverage scope, focused suites, native fixtures, and browser-test boundaries. |
| [Backend coverage gaps](todo/coverage-gaps.md) | Dated coverage shortfall, untested failure paths, instrumentation uncertainty, and reproduction commands. |
| [Tracing](tracing.md) | Collector/Jaeger setup, trace context, span meanings, configuration, and diagnostic commands. |
| [Photo performance](photo-performance.md) | Opt-in large-image benchmark, generated fixtures, report format, and local baseline comparison. |
| [Development tile requests](osm-tile-policy.md) | Tile URL, browser caching/referrer/credential settings, attribution, and development-only scope. |
| [Image fixture reference](../crates/api/tests/fixtures/README.md) | Provenance and use of the committed synthetic HEIC fixtures. |

## Repository entry points

| Document | Purpose |
| --- | --- |
| [Repository README](../README.md) | Product summary, source layout, stack, and documentation links. |
| [Web README](../web/README.md) | Frontend startup, proxy behavior, styling inputs, and check commands. |
| [Contributor guide](../AGENTS.md) | Repository constraints, communication and writing guidance, test-first delivery, and coverage requirements. |
| [This inventory](documentation-inventory.md) | Index of maintained Markdown documents. |

## Supporting implementation references

| Artifact | Purpose |
| --- | --- |
| [`config.toml`](../config.toml), [`.env.example`](../.env.example) | Configuration defaults and environment-variable reference. |
| [`Dockerfile`](../Dockerfile), [`docker-compose.yml`](../docker-compose.yml) | API image recipe and local service definitions. |
| [`dev/otel-collector.yaml`](../dev/otel-collector.yaml) | Development OTLP processing/export configuration. |
| [`design/tokens.css`](../design/tokens.css), [`design/components.css`](../design/components.css) | Runtime visual tokens and component styles imported by the application. |
| [`Cargo.toml`](../Cargo.toml), [`crates/api/Cargo.toml`](../crates/api/Cargo.toml), [`web/package.json`](../web/package.json) | Build profiles, dependency requirements, and executable commands. |
| [`.github/workflows/checks.yml`](../.github/workflows/checks.yml) | Automated frontend/backend checks and coverage thresholds. |
| [`web/.prettierrc.json`](../web/.prettierrc.json), [`web/.prettierignore`](../web/.prettierignore) | Frontend formatting rules and generated-file exclusions. |
| [`.cargo/config.toml`](../.cargo/config.toml), [`web/vitest.config.ts`](../web/vitest.config.ts) | Coverage commands, inclusion rules, and thresholds. |
| [`web/vite.config.ts`](../web/vite.config.ts) | Frontend aliases, development proxy, and map bundling configuration. |
| [`crates/api/migrations/`](../crates/api/migrations), [`.sqlx/`](../.sqlx) | Ordered schema changes and offline checked-query metadata. |
| [`web/src/api/types.ts`](../web/src/api/types.ts), [`web/src/api/__fixtures__/`](../web/src/api/__fixtures__) | Frontend response types and representative API contracts checked by tests. |

Generated coverage, build output, dependencies, local traces, and benchmark outputs are
generated files outside this documentation inventory.
