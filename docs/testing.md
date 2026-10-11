# Tests and coverage

The backend has Rust unit tests, PostgreSQL-backed API integration tests, native
image regression fixtures, and trace/export tests. Frontend tests use Vitest,
Testing Library, jsdom, and a MapLibre substitute. JSON fixture contracts connect
backend response examples to handwritten frontend types.

## Commands

Start PostgreSQL and install native dependencies as described in
[running locally](running.md). From the repository root:

```sh
export DATABASE_URL='postgres://pinvoyager:pinvoyager@localhost:5432/pinvoyager'
export SQLX_OFFLINE=true
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo coverage -- --test-threads=1

npm --prefix web run format:check
npm --prefix web run typecheck
npm --prefix web run lint
npm --prefix web run test -- --coverage
npm --prefix web run build
```

`cargo coverage` is the alias in `.cargo/config.toml` and requires
`cargo-llvm-cov`. It fails below 100% lines, functions, or regions. Stable Rust
does not provide the nightly branch instrumentation; meaningful conditionals
require tests for both outcomes. Frontend coverage requires 100% statements,
branches, functions, and lines across production TypeScript modules. Its
exclusions cover declarations, tests, fixtures, mocks, and test setup, not
production features. No production instrumentation exclusion is configured.

## Focused suites

Test filenames without a directory are relative to `crates/api/tests/`, except
`telemetry_tests.rs`, which is in `crates/api/src/`.

| Area | Command or tests | Behavior exercised |
| --- | --- | --- |
| Upload and readiness | `npm --prefix web run test:regression` | Creation validation, per-file state, transfer failures, pending-to-ready polling, and first voyage visit. |
| Native images | `crates/api/src/http/photos_tests.rs`, `crates/api/tests/photos_recovery.rs` | HEIC ingestion/conversion, corrupt inputs, variants, and pending recovery. |
| Privacy and ownership | `crates/api/tests/photo_ownership.rs`, `photos_public_access.rs`, `voyages_granularity.rs`, `discover_map.rs` | Voyage membership, draft visibility, authorized variants, rounding, and discovery data. |
| Authentication and names | `auth_flow.rs`, `auth_tokens.rs`, `display_names.rs` | Provider flow/session contracts, identity linkage, unique names, aliases, and public profiles. |
| API shapes | `api_fixtures.rs`, `web/src/api/__tests__/contract.test.ts` | Representative response fixtures and frontend type compatibility. |
| Tracing | `image_tracing.rs`, `telemetry_tests.rs` | Request/worker context, span attribution, export configuration, and shutdown. |
| Image performance | [Photo benchmark](photo-performance.md) | Opt-in large-image timing and saved-baseline comparisons. |

Native HEIC decode/probe tests use committed RGB/RGBA fixtures, so they require a
decoder but no encoder plugin. Photo fixture provenance is documented in
[`crates/api/tests/fixtures/README.md`](../crates/api/tests/fixtures/README.md). The
large-image performance test is ignored during ordinary test and coverage runs; its
tests of span attribution and timing comparisons run in the ordinary suite.

## What automated component tests establish

Component tests check state, DOM output, accessibility labels, focus-restoration calls,
query behavior, and configured map options. The substitutes do not render WebGL, fetch
real tiles, place dialogs above page content as a browser does, or verify native image
decoding and clipboard permissions. Check these behaviors in a real browser and test
provider sign-in with live credentials; component tests do not verify them. Backend
integration tests require PostgreSQL and the native image libraries.

## Fixtures, configuration, and CI

Ordinary `cargo test --test api_fixtures` compares real responses with committed
JSON after normalizing generated IDs and timestamps; it does not rewrite files.
To refresh the contract intentionally, run:

```sh
PINVOYAGER_UPDATE_FIXTURES=1 cargo test --test api_fixtures
npm --prefix web run test -- src/api/__tests__/contract.test.ts
```

Review the JSON changes. SQL metadata is separate: after checked queries change,
run `cargo sqlx prepare --workspace -- --all-targets` against a migrated database.
Config unit tests use explicit Figment sources, independent of the shell's
application environment. Signal tests send SIGINT/SIGTERM to a child process
after its readiness message; they do not signal the shared test runner.

`projectionBoundary.test.ts` compares geometry against the installed MapLibre
transform and Mercator projection. It does not require WebGL. Component tests
continue to use the map substitute for DOM and interaction checks.

`.github/workflows/checks.yml` runs web formatting, types, lint, coverage, and
build, plus Rust formatting, Clippy, and database/native-image coverage on stable
Rust. CI retains coverage reports as workflow artifacts. Generated reports are
ignored and are not maintained documentation.
