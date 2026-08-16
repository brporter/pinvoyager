# Phase 2 — Backend foundation

**Status:** not started
**Depends on:** nothing (can run parallel to phase 1)
**Owner:** Claude

## Goal

A Rust service that starts, reads configuration, connects to PostgreSQL, applies migrations,
serves a health endpoint, and returns structured errors — with the project conventions
(error type, tracing, testing, layout) established well enough that phases 3–5 add features
without re-deciding them.

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
- **T2** — Ctrl-C drains in-flight requests rather than dropping them.
- **T3** — Removing a required environment variable causes a startup failure naming the
  variable. Printing the config struct does not reveal any secret value.
- **T4** — A handler that returns an internal error produces a 500 with a generic body and a
  request ID, while the log line carries the real cause.
- **T6/T7** — `sqlx migrate run` creates both tables; a `\d identities` shows the unique
  constraint on `(provider, subject)`.
- **T8** — `/health` returns 200 with the database stopped; `/ready` returns 503.
- **T10** — `cargo test` passes from a clean checkout with only docker-compose running.
- **T6** — With the pool set to a deliberately tiny `max_connections`, concurrent requests
  beyond that queue and then return 503 on timeout. They do not hang. Verify this rather than
  assuming it — the default is no timeout, and the failure mode is indefinite hanging.
- **T11** — `tokio-console` runs against a dev build and shows tasks. Confirm the tooling
  works *now*, while the service is simple enough that anything it reports is obviously
  correct; it is much harder to trust a new profiler while chasing a real problem.

## Open questions

- Config file format — TOML assumed.
- Whether to adopt `sqlx`'s compile-time query checking now. It requires either a live
  database or a checked-in `.sqlx` offline cache during builds, which affects CI setup.

## Notes

The `/health` vs `/ready` split matters more than it looks: conflating them means a database
blip causes orchestrators to kill otherwise-healthy processes instead of just routing around
them.

`users` and `identities` land here rather than in phase 3 so that migration ordering is
established early and phase 3 is purely about protocol work.
