# Phase 2 — Backend foundation

**Status:** not started
**Depends on:** nothing (can run parallel to phase 1)
**Owner:** Claude

## Goal

An ASP.NET Core service that starts, reads configuration, connects to PostgreSQL, applies
migrations, serves health endpoints, and returns structured errors — with the project
conventions (error handling, logging, testing, layout) established well enough that phases
3–5 add features without re-deciding them.

## Non-goals

- No authentication (phase 3), no uploads (phase 4), no voyages (phase 5).
- No frontend.
- No production deployment. Local development only.
- No auth-adjacent scaffolding written "for later" — build it in phase 3 with real
  requirements in hand.
- **No ASP.NET Core Identity.** See the note at the bottom; this is a decision, not an
  oversight.

## Targets

1. Solution with an ASP.NET Core Web API project targeting **.NET 10** (LTS) using **Minimal
   APIs**, plus a test project. `Nullable` and `TreatWarningsAsErrors` enabled in
   `Directory.Build.props` so they apply solution-wide.
2. Graceful shutdown on SIGINT/SIGTERM, draining in-flight requests. The generic host does
   this by default — the target is verifying it, and setting a sensible
   `ShutdownTimeout`.
3. Layered configuration — environment variables over `appsettings.{Environment}.json` over
   `appsettings.json` — bound to typed options classes with `IOptions<T>`, validated with data
   annotations and `.ValidateOnStart()` so a missing or malformed setting fails at boot rather
   than on first use. Secrets come from user-secrets in development and environment variables
   elsewhere, never from a committed file.
4. Global exception handling via `IExceptionHandler` returning **RFC 9457 Problem Details**,
   mapping domain errors to status codes, and — importantly — never leaking internal detail to
   the client while logging the full exception server-side. The `traceId` goes in the response
   so a user-visible reference ID can be quoted back.
5. Structured JSON logging with `Microsoft.Extensions.Logging`, plus **OpenTelemetry** tracing
   with ASP.NET Core and Npgsql instrumentation. Request correlation rides on the W3C trace
   context that ASP.NET Core already propagates.
6. PostgreSQL via **Npgsql** and **EF Core**, with pooling and a `DbContext` registered through
   `AddDbContextPool`.
7. Initial EF Core migration creating `Users` and `Identities` per D3, with a unique index on
   `(Provider, Subject)`. No credential columns.
8. `GET /health` (liveness) and `GET /ready` (readiness) using
   `Microsoft.Extensions.Diagnostics.HealthChecks`, separated by **tag** so readiness includes
   the database check and liveness does not.
9. `docker-compose.yml` bringing up PostgreSQL for local development.
10. Test project using **xUnit** + `WebApplicationFactory<Program>` + **Testcontainers** for an
    ephemeral PostgreSQL instance, with an integration test asserting on `/health` and `/ready`.

## Acceptance criteria

- **T1** — `dotnet build` produces zero warnings; `dotnet format --verify-no-changes` passes.
- **T2** — Ctrl-C drains in-flight requests rather than dropping them.
- **T3** — Removing a required setting causes a startup failure naming the setting. Options
  classes holding secrets do not surface them in logs or diagnostics output.
- **T4** — A handler that throws produces a 500 with a Problem Details body containing a
  `traceId` and no internal detail, while the log carries the full exception.
- **T6/T7** — `dotnet ef database update` creates both tables; `\d "Identities"` shows the
  unique index on `(Provider, Subject)`.
- **T8** — `/health` returns 200 with the database stopped; `/ready` returns 503.
- **T10** — `dotnet test` passes from a clean checkout with Docker available. Testcontainers
  starts and disposes the database itself, so no manual setup step exists.

## Open questions

- **EF Core or Dapper.** EF Core is assumed because it brings migrations, which is a real
  chunk of this phase. Dapper is leaner and closer to hand-written SQL but needs a separate
  migration tool (DbUp or FluentMigrator). A reasonable middle path is EF Core for schema and
  writes, Dapper for the few read paths that turn out to be hot — decide when there is a hot
  path, not now.
- **Minimal APIs or controllers.** Minimal APIs assumed. This service is a small set of
  endpoints and does not need controller ceremony, but if the endpoint count grows past
  roughly thirty, revisit.
- Whether to add `Microsoft.AspNetCore.OpenApi` now or in phase 6. Adding it now costs little
  and phase 6 wants a generated client from it.

## Notes

**Do not use ASP.NET Core Identity.** It is the obvious-looking choice and it is wrong here.
Identity exists to manage local credentials — password hashes, lockout counters, two-factor
secrets, recovery codes — which is precisely what D1 says this system never stores. Phase 3
uses the external authentication handlers with cookie authentication and nothing else. Pulling
in Identity would add a schema full of credential columns that must stay permanently empty,
which is worse than useless: it is a standing invitation for someone later to start filling it.

**The `/health` versus `/ready` split matters more than it looks.** Conflating them means a
database blip causes orchestrators to kill otherwise-healthy processes instead of just routing
around them. The health check tags are what keep them separate.

`Users` and `Identities` land here rather than in phase 3 so migration ordering is established
early and phase 3 is purely about protocol work.

**Nullable reference types are on from the first commit.** Turning them on later in a codebase
of any size is a miserable, low-value migration, and they are the single biggest correctness
win available in modern C#.
