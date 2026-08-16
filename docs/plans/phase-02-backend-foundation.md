# Phase 2 — Backend foundation

**Status:** in progress — blocked on Docker
**Depends on:** nothing (can run parallel to phase 1)
**Owner:** Claude

Code for every target is written and the build is green: `cargo fmt --check`, `cargo clippy
--all-targets -- -D warnings`, and `cargo test` all pass (10 unit tests, 1 doctest, 1 ignored).

**Not yet verified, all for the same reason — the Docker daemon is not running:**
T9 (compose has never been brought up), T10 (no integration test against a real database),
T11 (tokio-console unrun), the T6 pool-exhaustion 503, the T7 schema inspection, and the T8
health/ready split against a database stopped after startup. The `.sqlx` offline cache also
cannot be generated, so queries remain runtime-checked rather than compile-time checked.

## Goal

A Rust service that starts, reads configuration, connects to PostgreSQL, applies migrations,
serves a health endpoint, and returns structured errors — with the project conventions
(error type, tracing, testing, layout) established well enough that phases 3–5 add features
without re-deciding them. A matching Docker container for the service as well as a compose file 
for dependencies (like PostgreSQL). Queries are checked at compile time using a .sqlx offline cache.

## Non-goals

- No authentication (phase 3), no uploads (phase 4), no voyages (phase 5).
- No frontend.
- No production deployment. Local development only.
- No auth-adjacent scaffolding written "for later" — build it in phase 3 with real
  requirements in hand.

## Targets

1. Cargo workspace with the service crate; builds clean under `cargo clippy -- -D warnings`.
2. axum server with graceful shutdown on SIGINT/SIGTERM.
3. Layered configuration — environment variables over a file over defaults — with a typed
   config struct that fails loudly at startup on anything missing or malformed. Secrets never
   logged, and a `Debug` impl that cannot accidentally print one.
4. A single application error type implementing `IntoResponse`, mapping cleanly to status
   codes, returning a consistent JSON error body, and — importantly — never leaking internal
   detail to the client while still logging the full cause server-side.
5. `tracing` with structured JSON output, a request-ID middleware, and per-request spans.
6. PostgreSQL via sqlx on a `PgPool`, **every query async** per D8, plus `sqlx::migrate!`
   wired to `migrations/`. Pool configured deliberately rather than by default:
   `max_connections` sized against Postgres's own limit, and **`acquire_timeout` always set**
   so exhaustion surfaces as a 503 instead of requests hanging forever.
7. An initial migration creating `users` and `identities` per D3. No credential columns.
8. `GET /health` (liveness) and `GET /ready` (readiness, verifies the database round-trips).
9. `docker-compose.yml` bringing up PostgreSQL for local development.
10. Test harness: an integration test that boots the app against a real ephemeral database
    and asserts on `/health` and `/ready`.
11. **Runtime-blocking discipline established now, before phase 4 needs it** (D8). A documented
    convention for where CPU-bound and blocking work goes — `tokio::task::spawn_blocking`, with
    a thin helper so call sites are uniform — plus `tokio-console` wired up behind a dev feature
    flag. Phase 4 introduces image processing, which is where this stops being theoretical;
    having the pattern already in place means it gets used rather than retrofitted.
12. A repository or data-access layer whose functions are all `async fn` taking
    `&PgPool` or an executor, so no call site can acquire a connection and forget to release
    it, and so tests can pass a transaction that rolls back.

## Acceptance criteria

- **T1** — `cargo clippy --all-targets -- -D warnings` is clean; `cargo fmt --check` passes.
- **T2** — **SIGTERM** drains in-flight requests rather than dropping them, and a request in
  flight when the signal arrives completes. Test with SIGTERM specifically, not Ctrl-C: SIGINT
  is what a developer sends and SIGTERM is what every container runtime sends, so a
  SIGINT-only implementation passes a Ctrl-C test and still severs connections in production.
  Also confirm the drain deadline bounds a wedged handler — `axum::serve`'s graceful shutdown
  waits indefinitely on its own.
- **T3** — Removing a required setting from **both** `config.toml` and the environment causes
  a startup failure naming it, in both its TOML spelling and its env-var spelling. (The
  checked-in `config.toml` supplies a working development database URL, so removing only the
  environment variable proves nothing.) Printing the config struct does not reveal any secret
  value.
- **T4** — A handler that returns an internal error produces a 500 with a generic body and a
  request ID, while the log line carries the real cause.
- **T5** — A request emits a span carrying its request id, every log line within that request
  carries the same id, and the id is returned to the client on `x-request-id`. JSON output
  parses as one object per line.
- **T6** — With the pool set to a deliberately tiny `max_connections`, concurrent requests
  beyond that queue and then return 503 on timeout. They do not hang. Verify this rather than
  assuming it — the default is no timeout, and the failure mode is indefinite hanging.
- **T7** — `sqlx migrate run` creates both tables; `\d identities` shows the unique constraint
  on `(provider, subject)`. Inspect the schema and confirm no column holds a password, hash,
  or token — D1 is verified by looking, not by intent.
- **T8** — With the service already running, stop the database: `/health` continues to return
  200 and `/ready` returns 503. **Order matters** — a service *started* without a database
  exits during migration and never serves either endpoint, which is correct behaviour but
  tests the wrong thing.
- **T9** — `docker compose up` brings PostgreSQL to a healthy state, and the service connects
  to it with no further setup.
- **T10** — `cargo test` passes from a clean checkout with only docker-compose running.
- **T12** — Repository functions accept an executor, so a test can pass a transaction and roll
  it back rather than cleaning up after itself. Demonstrate with at least one test.
- **T11** — `tokio-console` runs against a dev build and shows tasks. Confirm the tooling
  works *now*, while the service is simple enough that anything it reports is obviously
  correct; it is much harder to trust a new profiler while chasing a real problem.
  **The feature flag alone is not enough**: tokio only emits task instrumentation under
  `--cfg tokio_unstable`, so the run command is
  `RUSTFLAGS="--cfg tokio_unstable" cargo run --features tokio-console`. This is deliberately
  not set globally in `.cargo/config.toml` — that would apply the unstable cfg to release
  builds, including the container image.

## Open questions

- Config file format — TOML assumed.
- **Migrations run at startup for local convenience, and phase 8 moves them to a deploy step**
  (running them at boot races across instances). There is currently no config flag to disable
  startup migration, so phase 8 has to add one. Noted here so it is not discovered during a
  deploy.

## Notes

The `/health` vs `/ready` split matters more than it looks: conflating them means a database
blip causes orchestrators to kill otherwise-healthy processes instead of just routing around
them.

`users` and `identities` land here rather than in phase 3 so that migration ordering is
established early and phase 3 is purely about protocol work.
